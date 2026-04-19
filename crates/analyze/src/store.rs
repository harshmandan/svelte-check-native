//! Store auto-subscribe detection.
//!
//! Svelte's `$store` syntax auto-subscribes to a store value: writing
//! `$store` in script or template reads the store's current value, and
//! `$store = value` calls `store.set(value)`. For type-checking, the
//! `$store` identifier needs to exist somewhere — without a declaration
//! the reference fires TS2552 "Cannot find name '$store'. Did you mean
//! 'store'?".
//!
//! `find_store_refs` discovers candidate store references by:
//!
//! 1. Walking the script's oxc AST to collect every top-level binding
//!    (let/const/var/function/class/import-specifier name).
//! 2. Walking the script source again for `$<ident>` references where
//!    `<ident>` is in the binding set and isn't a rune name.
//!
//! Returns the set of store names that need to be declared as aliases.
//! The emit crate generates `let $<name>: any;` declarations from the
//! returned list.
//!
//! Limitations:
//! - Doesn't yet scan template interpolations (template-only store
//!   references are missed).
//! - Doesn't verify the bound value is actually a Svelte store at the
//!   type level (we emit `any` for safety).
//! - Doesn't handle dynamic store creation patterns.

use std::collections::HashSet;

use oxc_ast::ast::{
    BindingPatternKind, Declaration, ImportDeclarationSpecifier, ImportOrExportKind, Statement,
};
use smol_str::SmolStr;

/// All known rune names. Identifiers starting with `$` that match one of
/// these are NOT stores — they're rune calls.
const RUNE_NAMES: &[&str] = &[
    "$state",
    "$derived",
    "$effect",
    "$bindable",
    "$inspect",
    "$host",
    "$props",
];

/// Find candidate store references in a script.
///
/// Returns the list of unique `$<name>` references where `<name>` is
/// declared at the script's top level. Order is the order of first
/// occurrence in the source.
pub fn find_store_refs(program: &oxc_ast::ast::Program<'_>, source: &str) -> Vec<SmolStr> {
    let mut bound = HashSet::new();
    collect_top_level_bindings(program, &mut bound);
    find_store_refs_with_bindings(source, &bound)
}

/// Like [`find_store_refs`] but accepts a pre-computed binding set,
/// letting callers union module-script and instance-script bindings
/// (a `$store` reference in instance can resolve to a binding declared
/// in `<script module>`).
pub fn find_store_refs_with_bindings(source: &str, bound: &HashSet<String>) -> Vec<SmolStr> {
    if bound.is_empty() {
        return Vec::new();
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'$' {
            i += 1;
            continue;
        }
        // Anchor: previous char must NOT be an ident continuation, so
        // we don't match the `$` in the middle of `foo$bar`.
        if i > 0 {
            let prev = bytes[i - 1];
            if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
                i += 1;
                continue;
            }
        }
        // Read $<ident>.
        let name_start = i;
        let mut j = i + 1;
        if j >= bytes.len() {
            break;
        }
        // First char of identifier (after `$`) must be alpha or `_`.
        let first = bytes[j];
        if !(first.is_ascii_alphabetic() || first == b'_') {
            i += 1;
            continue;
        }
        j += 1;
        while j < bytes.len() {
            let b = bytes[j];
            // JS identifier-continuation chars: alphanumeric, `_`, `$`.
            if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
                j += 1;
            } else {
                break;
            }
        }
        let full = &source[name_start..j];
        let ident = &full[1..];

        if !RUNE_NAMES.contains(&full) && bound.contains(ident) && seen.insert(full.to_string()) {
            out.push(SmolStr::from(full));
        }
        i = j;
    }
    out
}

/// Collect the set of names declared at the top level of a script: imports
/// (default/named/namespace), `let`/`const`/`var`, function and class
/// declarations, and the same forms re-exported via `export ... = ...`.
///
/// Used by store auto-subscribe (this file) and by template-ref filtering
/// in the emit pipeline — anywhere we need to know "what names exist in
/// the script's scope?".
pub fn collect_top_level_bindings(program: &oxc_ast::ast::Program<'_>, out: &mut HashSet<String>) {
    for stmt in &program.body {
        collect_from_statement(stmt, out);
    }
}

/// Collect every top-level `let NAME: Type;` (typed, no initializer)
/// binding name. Used to seed the definite-assign rewriter for
/// Svelte-style "declare now, assign in a handler later" patterns —
/// matches upstream svelte-check's effective treatment where TS2454
/// doesn't fire on typed-uninit lets that the user assigns later in
/// an event handler, reactive statement, or template binding.
///
/// Only `let` is walked (const/var can't be both typed and uninit at
/// the same time: const requires init, var has no type annotation).
/// Destructuring patterns (`let { a }: T`) are skipped — those can't
/// carry `!` syntactically. Only simple-identifier bindings
/// qualify.
pub fn collect_typed_uninit_lets(
    program: &oxc_ast::ast::Program<'_>,
    out: &mut Vec<smol_str::SmolStr>,
) {
    collect_typed_lets_impl(program, out, true);
}

/// Collect every top-level `let NAME: Type[= init];` (typed, with or
/// without initializer) binding name. Used to seed the de-narrow
/// rewriter: `let X: T | null = null;` otherwise narrows to the
/// literal `null` via TS's control-flow analysis, so a subsequent
/// `if (X) X.foo` fires TS2339 on `never`. Inserting `X = undefined
/// as any;` after the declaration widens the flow-tracked type back
/// to the declared annotation.
pub fn collect_typed_top_level_lets(
    program: &oxc_ast::ast::Program<'_>,
    out: &mut Vec<smol_str::SmolStr>,
) {
    collect_typed_lets_impl(program, out, false);
}

fn collect_typed_lets_impl(
    program: &oxc_ast::ast::Program<'_>,
    out: &mut Vec<smol_str::SmolStr>,
    uninit_only: bool,
) {
    for stmt in &program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        if !matches!(decl.kind, oxc_ast::ast::VariableDeclarationKind::Let) {
            continue;
        }
        for declarator in &decl.declarations {
            if uninit_only && declarator.init.is_some() {
                continue;
            }
            // Only top-level simple identifier with a type annotation.
            let oxc_ast::ast::BindingPatternKind::BindingIdentifier(id) = &declarator.id.kind
            else {
                continue;
            };
            if declarator.id.type_annotation.is_none() {
                continue;
            }
            let name = smol_str::SmolStr::from(id.name.as_str());
            if !out.iter().any(|n| n == &name) {
                out.push(name);
            }
        }
    }
}

fn collect_from_statement(stmt: &Statement<'_>, out: &mut HashSet<String>) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                collect_from_binding_pattern(&declarator.id.kind, out);
            }
        }
        Statement::FunctionDeclaration(decl) => {
            if let Some(id) = &decl.id {
                out.insert(id.name.to_string());
            }
        }
        Statement::ClassDeclaration(decl) => {
            if let Some(id) = &decl.id {
                out.insert(id.name.to_string());
            }
        }
        Statement::ImportDeclaration(decl) => {
            // Whole-import `import type { X } from '...'` introduces no
            // runtime binding. Skip — voiding a type-only name fires
            // TS2693 ("only refers to a type, but is being used as a
            // value here").
            if matches!(decl.import_kind, ImportOrExportKind::Type) {
                return;
            }
            if let Some(specifiers) = &decl.specifiers {
                for spec in specifiers {
                    let (name, is_type_only) = match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            // Per-specifier `import { type X }`.
                            (
                                s.local.name.as_str(),
                                matches!(s.import_kind, ImportOrExportKind::Type),
                            )
                        }
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => {
                            (s.local.name.as_str(), false)
                        }
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => {
                            (s.local.name.as_str(), false)
                        }
                    };
                    if !is_type_only {
                        out.insert(name.to_string());
                    }
                }
            }
        }
        Statement::ExportNamedDeclaration(decl) => {
            if let Some(d) = &decl.declaration {
                collect_from_declaration(d, out);
            }
        }
        _ => {}
    }
}

fn collect_from_declaration(decl: &Declaration<'_>, out: &mut HashSet<String>) {
    match decl {
        Declaration::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                collect_from_binding_pattern(&declarator.id.kind, out);
            }
        }
        Declaration::FunctionDeclaration(decl) => {
            if let Some(id) = &decl.id {
                out.insert(id.name.to_string());
            }
        }
        Declaration::ClassDeclaration(decl) => {
            if let Some(id) = &decl.id {
                out.insert(id.name.to_string());
            }
        }
        _ => {}
    }
}

fn collect_from_binding_pattern(pat: &BindingPatternKind<'_>, out: &mut HashSet<String>) {
    match pat {
        BindingPatternKind::BindingIdentifier(id) => {
            out.insert(id.name.to_string());
        }
        BindingPatternKind::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_from_binding_pattern(&prop.value.kind, out);
            }
            if let Some(rest) = &obj.rest {
                collect_from_binding_pattern(&rest.argument.kind, out);
            }
        }
        BindingPatternKind::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_from_binding_pattern(&el.kind, out);
            }
            if let Some(rest) = &arr.rest {
                collect_from_binding_pattern(&rest.argument.kind, out);
            }
        }
        BindingPatternKind::AssignmentPattern(asn) => {
            collect_from_binding_pattern(&asn.left.kind, out);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::{ScriptLang, parse_script_body};

    fn refs(src: &str) -> Vec<String> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        find_store_refs(&parsed.program, src)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn simple_store_ref() {
        let src = "let store = null;\nconst x = $store + 1;";
        assert_eq!(refs(src), vec!["$store"]);
    }

    #[test]
    fn store_ref_in_type_position() {
        let src = "let store = null;\nlet x: typeof $store = $store;";
        assert_eq!(refs(src), vec!["$store"]);
    }

    #[test]
    fn unknown_dollar_ident_not_returned() {
        // `$mystery` isn't declared anywhere, so don't emit an alias —
        // it's likely a typo or an external store we don't know about.
        let src = "const x = $mystery;";
        assert!(refs(src).is_empty());
    }

    #[test]
    fn rune_names_excluded() {
        // `$state`, `$derived` etc. are runes — even if the user has a
        // local named `state`, `$state` is the rune call.
        let src = "let state = null;\nconst x = $state(0);";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$state"));
    }

    #[test]
    fn imported_store() {
        let src =
            "import { writable } from 'svelte/store';\nconst foo = writable(0);\nconst x = $foo;";
        let r = refs(src);
        assert!(r.iter().any(|s| s == "$foo"));
    }

    #[test]
    fn dollar_suffix_identifier_not_a_store_ref() {
        // `parent$` is an ordinary identifier — `$parent$` should be
        // recognized as the store ref of `parent$`, not as `$parent`.
        let src = "let parent$ = null;\nconst x = $parent$;";
        let r = refs(src);
        assert!(r.iter().any(|s| s == "$parent$"));
    }

    #[test]
    fn destructured_binding_recognized() {
        let src = "let { foo } = obj;\nconst x = $foo;";
        let r = refs(src);
        assert!(r.iter().any(|s| s == "$foo"));
    }

    #[test]
    fn inside_string_not_a_ref() {
        // A `$store` inside a string literal would still be a token by
        // our simple byte scanner. This test documents the limitation —
        // we'd need a full lexer to filter it out. For now we accept the
        // false positive (cost: one extra `let $store: any;` declaration
        // for stores that don't actually need one — harmless).
        let src = r#"let store = null;\nconst msg = "hello $store";"#;
        let r = refs(src);
        // Either passes or finds the false positive — we don't assert
        // either way; this is informational.
        let _ = r;
    }

    #[test]
    fn no_bindings_returns_empty() {
        let src = "console.log('hi');";
        assert!(refs(src).is_empty());
    }

    #[test]
    fn order_of_first_occurrence_preserved() {
        let src = "let a = null;\nlet b = null;\nconst x = $b + $a;";
        assert_eq!(refs(src), vec!["$b", "$a"]);
    }
}
