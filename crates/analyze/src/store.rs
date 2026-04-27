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
//! The scanner skips `// line` and `/* block */` comments, single-
//! and double-quoted strings, and the static segments of template
//! literals. Interpolations (`${…}`) are re-scanned as normal code,
//! so a `$store` inside a `${…}` IS picked up. Regex literals are
//! not specially handled — in practice a regex containing a pattern
//! that happens to match a bound script name is rare, and the
//! intersection-with-bindings filter keeps the scanner conservative.
//!
//! Limitations:
//! - Doesn't yet scan template interpolations (template-only store
//!   references are missed).
//! - Doesn't verify the bound value is actually a Svelte store at the
//!   type level (we emit `any` for safety).
//! - Doesn't handle dynamic store creation patterns.

use std::collections::HashSet;

use oxc_ast::ast::{
    BindingPattern, Declaration, ImportDeclarationSpecifier, ImportOrExportKind, Statement,
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
    // Template-literal depth stack: each entry is the brace-nesting
    // count at which we re-enter template-quoted mode when the brace
    // count drops back to 0. Non-empty when inside `${…}` of a
    // template literal. Lets nested templates work correctly.
    let mut template_stack: Vec<u32> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];

        // Line comment.
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // Single- or double-quoted string. `$ident` inside is a
        // literal substring, not a store reference — skip the whole
        // thing.
        if b == b'"' || b == b'\'' {
            let quote = b;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else if bytes[i] == b'\n' {
                    // Unterminated; bail conservatively to avoid a
                    // runaway skip if the user's code is mid-edit.
                    break;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1; // closing quote
            }
            continue;
        }
        // Template literal open. Skip the static prefix; `${…}`
        // interpolations push a stack entry so the inner expression
        // goes through normal scanning (a `$store` inside `${…}`
        // IS a real store ref) and we return to template-quoted
        // mode when the matching `}` is seen.
        if b == b'`' {
            i += 1;
            while i < bytes.len() {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                    continue;
                }
                if bytes[i] == b'`' {
                    i += 1;
                    break;
                }
                if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
                    template_stack.push(0);
                    i += 2;
                    break;
                }
                i += 1;
            }
            continue;
        }
        // Brace accounting while we're in the expression part of a
        // template literal.
        if !template_stack.is_empty() {
            if b == b'{' {
                if let Some(depth) = template_stack.last_mut() {
                    *depth += 1;
                }
                i += 1;
                continue;
            }
            if b == b'}' {
                if let Some(depth) = template_stack.last_mut() {
                    if *depth == 0 {
                        // End of `${…}` — resume template-quoted mode
                        // for the current level.
                        template_stack.pop();
                        i += 1;
                        // Re-use the template-literal skip above by
                        // pretending we just hit a backtick boundary:
                        // fall through so the next iteration sees
                        // whatever character follows. But the rest of
                        // the template string still needs skipping,
                        // so walk until ` or another ${.
                        while i < bytes.len() {
                            if bytes[i] == b'\\' && i + 1 < bytes.len() {
                                i += 2;
                                continue;
                            }
                            if bytes[i] == b'`' {
                                i += 1;
                                break;
                            }
                            if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
                                template_stack.push(0);
                                i += 2;
                                break;
                            }
                            i += 1;
                        }
                        continue;
                    }
                    *depth -= 1;
                }
                i += 1;
                continue;
            }
        }

        if b != b'$' {
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

/// Collect the set of type-only import specifier names. Parallel to
/// [`collect_top_level_bindings`] but only returns names that were
/// imported strictly as types (`import type { X }` / `import { type X }`).
/// These have no runtime value — downstream emit must reference them in
/// TYPE position (`type _ = [X, Y]`) to keep TS from firing TS6133 when
/// they're only consumed inside template expressions (e.g. as cast
/// targets: `{foo(item as AppVideo)}`).
pub fn collect_type_only_import_bindings(
    program: &oxc_ast::ast::Program<'_>,
    out: &mut HashSet<String>,
) {
    for stmt in &program.body {
        let Statement::ImportDeclaration(decl) = stmt else {
            continue;
        };
        // `import type { X, Y } from '...'` — every specifier is type-only.
        if matches!(decl.import_kind, ImportOrExportKind::Type) {
            if let Some(specifiers) = &decl.specifiers {
                for spec in specifiers {
                    if let ImportDeclarationSpecifier::ImportSpecifier(s) = spec {
                        out.insert(s.local.name.to_string());
                    }
                }
            }
            continue;
        }
        // Mixed import with per-specifier `type` prefix.
        if let Some(specifiers) = &decl.specifiers {
            for spec in specifiers {
                if let ImportDeclarationSpecifier::ImportSpecifier(s) = spec {
                    if matches!(s.import_kind, ImportOrExportKind::Type) {
                        out.insert(s.local.name.to_string());
                    }
                }
            }
        }
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
    collect_typed_lets_impl(program, out, NarrowableFilter::UninitOnly);
}

#[derive(Copy, Clone)]
enum NarrowableFilter {
    UninitOnly,
    OnlyNullUndefined,
}

/// Collect every top-level `let NAME: Type = null;` / `let NAME: Type
/// = undefined;` binding — the subset where TS's control-flow analysis
/// narrows the variable's flow type to the literal (`null` /
/// `undefined`) on subsequent reads. These need the denarrow rewrite
/// so a later `if (X) X.foo` doesn't fire TS2339 on `never`.
///
/// Narrower than the old full-typed-let collection: `let num: number
/// = 0;` and `let flag: boolean = false;` don't need denarrow because
/// TS widens numeric/string/boolean literals to their primitive types
/// when assigned to a `let` binding (no `as const`). Only `null` and
/// `undefined` initializers stick as the narrow type.
///
/// This matters for v0.3 bind: contract checks: our inline emit
/// `void ((): void => { EXPR = null as any as TYPE; });` sees EXPR
/// as flow-narrowed after the previous denarrow `X = undefined as
/// any;` rewrite. For bind targets that AREN'T `null`/`undefined`-
/// initialized (the common case `let myRef: HTMLInputElement | null
/// = null` IS narrowable; `let num: number = 0` is NOT), skipping
/// the denarrow preserves the check's effectiveness.
pub fn collect_typed_top_level_lets(
    program: &oxc_ast::ast::Program<'_>,
    out: &mut Vec<smol_str::SmolStr>,
) {
    collect_typed_lets_impl(program, out, NarrowableFilter::OnlyNullUndefined);
}

fn collect_typed_lets_impl(
    program: &oxc_ast::ast::Program<'_>,
    out: &mut Vec<smol_str::SmolStr>,
    filter: NarrowableFilter,
) {
    for stmt in &program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        if !matches!(decl.kind, oxc_ast::ast::VariableDeclarationKind::Let) {
            continue;
        }
        for declarator in &decl.declarations {
            match filter {
                NarrowableFilter::UninitOnly => {
                    if declarator.init.is_some() {
                        continue;
                    }
                }
                NarrowableFilter::OnlyNullUndefined => {
                    // Must HAVE an initializer AND it must be
                    // literal null / identifier `undefined`.
                    let Some(init) = declarator.init.as_ref() else {
                        continue;
                    };
                    if !is_null_or_undefined_literal(init) {
                        continue;
                    }
                }
            }
            // Only top-level simple identifier with a type annotation.
            let oxc_ast::ast::BindingPattern::BindingIdentifier(id) = &declarator.id else {
                continue;
            };
            if declarator.type_annotation.is_none() {
                continue;
            }
            let name = smol_str::SmolStr::from(id.name.as_str());
            if !out.iter().any(|n| n == &name) {
                out.push(name);
            }
        }
    }
}

/// True when the expression is literal `null` or the `undefined`
/// identifier. These are the only primitive initializers TS narrows
/// to the literal type for `let NAME: T = init;` bindings — numeric/
/// string/boolean literals widen to their primitive types and don't
/// need the denarrow rewrite.
fn is_null_or_undefined_literal(expr: &oxc_ast::ast::Expression<'_>) -> bool {
    match expr {
        oxc_ast::ast::Expression::NullLiteral(_) => true,
        oxc_ast::ast::Expression::Identifier(id) => id.name == "undefined",
        _ => false,
    }
}

fn collect_from_statement(stmt: &Statement<'_>, out: &mut HashSet<String>) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                collect_from_binding_pattern(&declarator.id, out);
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
                collect_from_binding_pattern(&declarator.id, out);
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

fn collect_from_binding_pattern(pat: &BindingPattern<'_>, out: &mut HashSet<String>) {
    match pat {
        BindingPattern::BindingIdentifier(id) => {
            out.insert(id.name.to_string());
        }
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_from_binding_pattern(&prop.value, out);
            }
            if let Some(rest) = &obj.rest {
                collect_from_binding_pattern(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_from_binding_pattern(el, out);
            }
            if let Some(rest) = &arr.rest {
                collect_from_binding_pattern(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(asn) => {
            collect_from_binding_pattern(&asn.left, out);
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
    fn inside_double_quoted_string_not_a_ref() {
        let src = "let store = null;\nconst msg = \"hello $store\";";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn inside_single_quoted_string_not_a_ref() {
        let src = "let store = null;\nconst msg = 'hello $store';";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn inside_line_comment_not_a_ref() {
        let src = "let store = null;\n// see $store for details\nconst x = 1;";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn inside_block_comment_not_a_ref() {
        let src = "let store = null;\n/* uses $store here */\nconst x = 1;";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn inside_template_literal_static_part_not_a_ref() {
        let src = "let store = null;\nconst msg = `hello $store world`;";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn inside_template_interpolation_is_a_ref() {
        let src = "let store = null;\nconst msg = `x ${$store} y`;";
        let r = refs(src);
        assert!(r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn template_with_nested_object_expression_handles_braces() {
        // Braces in an interpolation must not prematurely close the
        // template-interpolation state.
        let src = "let store = null;\nconst msg = `v ${ { a: $store } } w`;";
        let r = refs(src);
        assert!(r.iter().any(|s| s == "$store"));
    }

    #[test]
    fn escaped_quote_does_not_end_string_early() {
        let src = "let store = null;\nconst msg = \"a \\\" $store \\\" b\";";
        let r = refs(src);
        assert!(!r.iter().any(|s| s == "$store"));
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
