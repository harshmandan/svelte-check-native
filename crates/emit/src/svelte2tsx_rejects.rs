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
//!   or alias (`processModuleScriptTag.ts`);
//! - template: a `<slot>` whose first attribute called `name` is a bare
//!   `name` or a directive (`slot.ts` `handleSlot` reads
//!   `value[0].raw` off a value that is `true` or missing);
//! - template: a direct child of a component whose first attribute
//!   called `slot` is a bare `slot` (`svelteAst.ts` `getSlotName`
//!   reads `.raw` off `true[0]`);
//! - template: a `let:` whose value is an object literal with a spread,
//!   on a component or on a component's direct child that names a slot
//!   (`handleScopeAndResolveForSlot.ts` hands the object to periscopic's
//!   `extract_identifiers`, which reads the missing `value` of the
//!   spread).

use oxc_ast::ast::{
    Declaration, Expression, Program, PropertyKey, Statement, TSInterfaceDeclaration, TSSignature,
    TSType, TSTypeAliasDeclaration, TSTypeName,
};
use oxc_ast_visit::{Visit, walk};
use svn_analyze::template_scope::{TemplateScopeVisitor, walk_with_visitor};
use svn_parser::{
    Attribute, Component, Document, Element, Fragment, Node, ParsedScript, ScriptSection,
    SvelteElement, SvelteElementKind,
};

/// Would svelte2tsx throw while converting this component?
pub(crate) fn svelte2tsx_rejects(
    doc: &Document<'_>,
    fragment: &Fragment,
    parsed_instance: Option<&ParsedScript<'_>>,
    parsed_module: Option<&ParsedScript<'_>>,
) -> bool {
    let mut template = TemplateProbe {
        source: doc.source,
        rejected: false,
    };
    walk_with_visitor(fragment, doc.source, &mut template);
    if template.rejected {
        return true;
    }
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

/// The first attribute written with `name` (a directive by the name
/// after its prefix), as svelte2tsx's `attributes.find` sees them.
fn first_named<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attributes.iter().find(|a| match a {
        Attribute::Plain(p) => p.name == name,
        Attribute::Expression(x) => x.name == name,
        Attribute::Shorthand(x) => x.name == name,
        Attribute::Directive(d) => d.name == name,
        Attribute::Spread(_) | Attribute::Comment(_) => false,
    })
}

struct TemplateProbe<'s> {
    source: &'s str,
    rejected: bool,
}

impl TemplateProbe<'_> {
    /// A component's own `let:` directives, and its direct children's
    /// slot names and (when they name a slot) `let:` directives.
    fn check_component(&mut self, attributes: &[Attribute], children: &Fragment) {
        if self.has_spreading_let(attributes) {
            self.rejected = true;
        }
        for child in &children.nodes {
            let attributes = match child {
                Node::Element(e) => &e.attributes,
                Node::Component(c) => &c.attributes,
                Node::SvelteElement(e) => &e.attributes,
                _ => continue,
            };
            if let Some(Attribute::Plain(p)) = first_named(attributes, "slot") {
                match &p.value {
                    None => self.rejected = true,
                    Some(v) => {
                        let named = matches!(
                            v.parts.first(),
                            Some(svn_parser::AttrValuePart::Text { range }) if range.start < range.end
                        );
                        if named && self.has_spreading_let(attributes) {
                            self.rejected = true;
                        }
                    }
                }
            }
        }
    }

    /// Does a `let:` directive's value read as an object literal with a
    /// spread?
    fn has_spreading_let(&self, attributes: &[Attribute]) -> bool {
        attributes.iter().any(|a| {
            let Attribute::Directive(d) = a else {
                return false;
            };
            let (svn_parser::DirectiveKind::Let, Some(svn_parser::DirectiveValue::Expression { expression_range, .. })) =
                (d.kind, &d.value)
            else {
                return false;
            };
            let Some(text) = self
                .source
                .get(expression_range.start as usize..expression_range.end as usize)
            else {
                return false;
            };
            let allocator = oxc_allocator::Allocator::default();
            let wrapped = format!("({text})");
            let parsed =
                svn_parser::parse_script_body(&allocator, &wrapped, svn_parser::ScriptLang::Ts);
            let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
                return false;
            };
            let mut expr = &stmt.expression;
            while let Expression::ParenthesizedExpression(p) = expr {
                expr = &p.expression;
            }
            matches!(
                expr,
                Expression::ObjectExpression(obj)
                    if obj.properties.iter().any(|p| matches!(p, oxc_ast::ast::ObjectPropertyKind::SpreadProperty(_)))
            )
        })
    }
}

impl TemplateScopeVisitor for TemplateProbe<'_> {
    fn visit_element(&mut self, element: &Element) {
        if element.name != "slot" {
            return;
        }
        let unreadable = match first_named(&element.attributes, "name") {
            Some(Attribute::Plain(p)) => p.value.is_none(),
            Some(Attribute::Directive(_)) => true,
            _ => false,
        };
        if unreadable {
            self.rejected = true;
        }
    }

    fn visit_component(&mut self, component: &Component) {
        self.check_component(&component.attributes, &component.children);
    }

    fn visit_svelte_element(&mut self, element: &SvelteElement) {
        if matches!(
            element.kind,
            SvelteElementKind::SelfRef | SvelteElementKind::Component
        ) {
            self.check_component(&element.attributes, &element.children);
        }
    }
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
        let (fragment, _) = svn_parser::parse_all_template_runs(src, &doc.template.text_runs);
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
        svelte2tsx_rejects(&doc, &fragment, instance.as_ref(), module.as_ref())
    }

    #[test]
    fn unreadable_slot_names() {
        assert!(rejects("<slot name></slot>"));
        assert!(rejects("<slot on:name></slot>"));
        assert!(rejects("{#if a}<slot bind:name={x}></slot>{/if}"));
        assert!(!rejects("<slot name=\"\"></slot><slot name={n}></slot>"));
        assert!(!rejects("<slot {name}></slot><slot foo></slot>"));
    }

    #[test]
    fn bare_slot_attribute_under_a_component() {
        assert!(rejects("<C><div slot></div></C>"));
        assert!(rejects("<C><C slot /></C>"));
        assert!(rejects("<C><svelte:fragment slot></svelte:fragment></C>"));
        assert!(rejects("<svelte:self><div slot></div></svelte:self>"));
        assert!(rejects(
            "<svelte:component this={C}><div slot></div></svelte:component>"
        ));
        assert!(!rejects("<C>{#if x}<div slot></div>{/if}</C>"));
        assert!(!rejects("<div><div slot></div></div>"));
        assert!(!rejects("<C><div slot=\"\"></div><div slot={x}></div></C>"));
        assert!(!rejects("<C><div on:slot slot></div></C>"));
    }

    #[test]
    fn object_spread_in_a_resolved_let() {
        assert!(rejects("<C let:item={{ a, ...rest }} />"));
        assert!(rejects(
            "<C><div slot=\"x\" let:item={{ ...rest }}></div></C>"
        ));
        assert!(!rejects("<C let:item={[a, ...rest]} />"));
        assert!(!rejects("<div><span let:item={{ ...rest }}></span></div>"));
        assert!(!rejects("<C><div let:item={{ ...rest }}></div></C>"));
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
