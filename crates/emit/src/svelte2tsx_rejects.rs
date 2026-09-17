//! Components svelte2tsx refuses to convert.
//!
//! svelte2tsx throws on a handful of script declarations it cannot
//! give a meaning to. svelte-check catches the throw, writes no overlay
//! for the file and relies on the Svelte compiler for any error
//! (`svelte-check/src/incremental.ts`), so such a file reports no
//! TypeScript diagnostics of its own, and importers see it only through
//! Svelte's `*.svelte` module wildcard. [`svelte2tsx_rejects`] tells
//! the caller to do the same.
//!
//! The throws, each checked on every node of the script, nested ones
//! included:
//!
//! - instance script: a `type X = $$Generic<…>` alias when the script
//!   tag also has a `generics` attribute, or when the alias passes more
//!   than one type argument (`nodes/Generics.ts`);
//! - instance script: a `$$Events` interface or alias naming an event
//!   by anything other than an identifier, a string literal, or a
//!   computed reference to a top-level string constant
//!   (`nodes/ComponentEvents.ts`);
//! - module script: a `generics` attribute on its tag, or any
//!   `$$Generic` alias or `$$Events` / `$$Slots` / `$$Props` interface
//!   or alias (`processModuleScriptTag.ts`).

use oxc_ast::ast::{
    Declaration, Expression, Program, PropertyKey, Statement, TSInterfaceDeclaration, TSSignature,
    TSType, TSTypeAliasDeclaration, TSTypeName,
};
use oxc_ast_visit::{Visit, walk};
use svn_parser::{Document, ParsedScript, ScriptSection};

/// Would svelte2tsx throw while converting this component?
pub(crate) fn svelte2tsx_rejects(
    doc: &Document<'_>,
    parsed_instance: Option<&ParsedScript<'_>>,
    parsed_module: Option<&ParsedScript<'_>>,
) -> bool {
    if doc.module_script.as_ref().is_some_and(has_generics_attr) {
        return true;
    }
    if let Some(module) = parsed_module {
        let mut probe = ModuleProbe { rejected: false };
        probe.visit_program(&module.program);
        if probe.rejected {
            return true;
        }
    }
    if let Some(instance) = parsed_instance {
        let mut probe = InstanceProbe {
            program: &instance.program,
            generics_attr: doc.instance_script.as_ref().is_some_and(has_generics_attr),
            rejected: false,
        };
        probe.visit_program(&instance.program);
        if probe.rejected {
            return true;
        }
    }
    false
}

/// A `generics` attribute with a non-empty value — a valueless or empty
/// one declares nothing and is ignored upstream.
fn has_generics_attr(script: &ScriptSection<'_>) -> bool {
    script
        .attrs
        .iter()
        .any(|a| a.name == "generics" && a.value.as_deref().is_some_and(|v| !v.is_empty()))
}

/// The `$$Generic` reference an alias is declared as, if any.
fn generic_alias_arity(alias: &TSTypeAliasDeclaration<'_>) -> Option<usize> {
    let TSType::TSTypeReference(reference) = &alias.type_annotation else {
        return None;
    };
    let TSTypeName::IdentifierReference(name) = &reference.type_name else {
        return None;
    };
    (name.name == "$$Generic").then(|| {
        reference
            .type_arguments
            .as_ref()
            .map_or(0, |args| args.params.len())
    })
}

struct ModuleProbe {
    rejected: bool,
}

impl<'a> Visit<'a> for ModuleProbe {
    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        if generic_alias_arity(it).is_some()
            || matches!(it.id.name.as_str(), "$$Events" | "$$Slots" | "$$Props")
        {
            self.rejected = true;
        }
        walk::walk_ts_type_alias_declaration(self, it);
    }

    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        if matches!(it.id.name.as_str(), "$$Events" | "$$Slots" | "$$Props") {
            self.rejected = true;
        }
        walk::walk_ts_interface_declaration(self, it);
    }
}

struct InstanceProbe<'p, 'a> {
    program: &'p Program<'a>,
    generics_attr: bool,
    rejected: bool,
}

impl<'a> Visit<'a> for InstanceProbe<'_, 'a> {
    fn visit_ts_type_alias_declaration(&mut self, it: &TSTypeAliasDeclaration<'a>) {
        if let Some(arity) = generic_alias_arity(it)
            && (self.generics_attr || arity > 1)
        {
            self.rejected = true;
        }
        if it.id.name == "$$Events" {
            let members: Vec<&TSSignature<'a>> = match &it.type_annotation {
                TSType::TSTypeLiteral(lit) => lit.members.iter().collect(),
                TSType::TSIntersectionType(i) => i
                    .types
                    .iter()
                    .filter_map(|t| match t {
                        TSType::TSTypeLiteral(lit) => Some(lit.members.iter()),
                        _ => None,
                    })
                    .flatten()
                    .collect(),
                _ => Vec::new(),
            };
            if members.into_iter().any(|m| !self.event_name_ok(m)) {
                self.rejected = true;
            }
        }
        walk::walk_ts_type_alias_declaration(self, it);
    }

    fn visit_ts_interface_declaration(&mut self, it: &TSInterfaceDeclaration<'a>) {
        if it.id.name == "$$Events" && it.body.body.iter().any(|m| !self.event_name_ok(m)) {
            self.rejected = true;
        }
        walk::walk_ts_interface_declaration(self, it);
    }
}

impl InstanceProbe<'_, '_> {
    /// Can upstream name the event a `$$Events` member declares? Only
    /// property signatures are read.
    fn event_name_ok(&self, member: &TSSignature<'_>) -> bool {
        let TSSignature::TSPropertySignature(prop) = member else {
            return true;
        };
        match (&prop.key, prop.computed) {
            (PropertyKey::StaticIdentifier(_) | PropertyKey::StringLiteral(_), false) => true,
            (PropertyKey::Identifier(id), true) => self.top_level_string_const(&id.name),
            _ => false,
        }
    }

    /// Is `name` a top-level variable of the script initialised with a
    /// string literal? The first top-level declaration of the name
    /// decides.
    fn top_level_string_const(&self, name: &str) -> bool {
        for stmt in &self.program.body {
            let decl = match stmt {
                Statement::VariableDeclaration(d) => d,
                Statement::ExportDeclaration(e) => match &e.declaration {
                    Declaration::VariableDeclaration(d) => d,
                    _ => continue,
                },
                _ => continue,
            };
            if let Some(found) = decl
                .declarations
                .iter()
                .find(|d| d.id.get_identifier_name().is_some_and(|n| n == name))
            {
                return matches!(found.init, Some(Expression::StringLiteral(_)));
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rejects(src: &str) -> bool {
        let (doc, _) = svn_parser::parse_sections(src);
        let alloc_i = oxc_allocator::Allocator::default();
        let alloc_m = oxc_allocator::Allocator::default();
        let instance = doc
            .instance_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc_i, s.content, s.lang));
        let module = doc
            .module_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc_m, s.content, s.lang));
        svelte2tsx_rejects(&doc, instance.as_ref(), module.as_ref())
    }

    #[test]
    fn generic_aliases() {
        assert!(!rejects(
            "<script lang=\"ts\">type T = $$Generic<string>;</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\">type T = $$Generic<A, B>;</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\" generics=\"U\">type T = $$Generic;</script>"
        ));
        assert!(!rejects(
            "<script lang=\"ts\" generics=\"\">type T = $$Generic;</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\">function f() { type T = $$Generic<A, B>; }</script>"
        ));
    }

    #[test]
    fn module_script_declarations() {
        assert!(rejects(
            "<script lang=\"ts\" context=\"module\">interface $$Props {}</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\" module>function f() { type $$Slots = {}; }</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\" module generics=\"T\"></script><script lang=\"ts\"></script>"
        ));
        assert!(!rejects(
            "<script lang=\"ts\" module>interface Props {}</script>"
        ));
    }

    #[test]
    fn event_names() {
        assert!(!rejects(
            "<script lang=\"ts\">interface $$Events { a: Event; 'b-c': Event; m(): void }</script>"
        ));
        assert!(!rejects(
            "<script lang=\"ts\">const K = 'k'; interface $$Events { [K]: Event }</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\">let K = 1; interface $$Events { [K]: Event }</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\">interface $$Events { ['k']: Event }</script>"
        ));
        assert!(rejects(
            "<script lang=\"ts\">type $$Events = Base & { 1: Event };</script>"
        ));
    }
}
