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
//! 2. Walking the same AST for `$<ident>` identifier references where
//!    `<ident>` is in the binding set and the occurrence isn't a rune.
//!
//! Returns the set of store names that need to be declared as aliases.
//! The emit crate generates `let $<name>: any;` declarations from the
//! returned list.
//!
//! Only identifier references count, mirroring upstream
//! `processInstanceScriptContent`: a member name (`api.$count`), an
//! object key (`{ $count: 1 }`), a type reference or anything inside a
//! type alias / interface is not a subscription. Comments, strings and
//! regex literals never reach the walk.
//!
//! Limitations:
//! - Doesn't yet scan template interpolations (template-only store
//!   references are missed).
//! - Doesn't verify the bound value is actually a Svelte store at the
//!   type level (we emit `any` for safety).
//! - Doesn't handle dynamic store creation patterns.

use std::collections::HashSet;

use oxc_ast::ast::{
    BindingPattern, Declaration, Expression, IdentifierReference, ImportDeclarationSpecifier,
    ImportOrExportKind, Program, Statement, VariableDeclaration,
};
use oxc_ast_visit::Visit;
use oxc_span::GetSpan;
use smol_str::SmolStr;

use crate::ast_walk::{WalkNode, walk_statement_descend};

/// Find candidate store references in a script.
///
/// Returns the list of unique `$<name>` references where `<name>` is
/// declared at the script's top level. Order is the order of first
/// occurrence in the source.
pub fn find_store_refs(program: &oxc_ast::ast::Program<'_>, source: &str) -> Vec<SmolStr> {
    let mut bound = HashSet::new();
    collect_top_level_bindings(program, &mut bound);
    let runes = collect_rune_scan_context(program, source);
    let mut refs = find_store_refs_with_bindings(program, source, &bound, &runes);
    // Mirrors upstream ImplicitStoreValues.isSvelteStoreDerivedImport:
    // a named import of `derived` from 'svelte/store' never gets a
    // store subscription in Svelte 5+ — `$derived(...)` in such a file
    // is the rune, not a subscription to the imported factory.
    if has_svelte_store_derived_import(program) {
        refs.retain(|r| r != "$derived");
    }
    refs
}

/// Rune names that upstream conditionally treats as rune usage rather
/// than a store subscription (processInstanceScriptContent.ts's
/// `is_rune` gate). Everything else — including `$effect`, `$bindable`,
/// `$inspect`, `$host` — is never special-cased there: those become
/// store subscriptions whenever the base name is a real binding.
const CONDITIONAL_RUNE_NAMES: &[&str] = &["$state", "$derived", "$props"];

/// Collect the byte offsets (of the leading `$`) of every `$state` /
/// `$derived` / `$props` identifier that sits in upstream's "is a rune,
/// not a store" position: a direct child (callee or argument) of a call
/// expression that is itself the initializer of a variable declarator
/// whose binding-pattern source text contains the rune's base name.
///
/// Mirrors upstream `processInstanceScriptContent.ts`:
///
/// ```ts
/// const is_rune =
///     (text === '$props' || text === '$derived' || text === '$state') &&
///     ts.isCallExpression(parent) &&
///     ts.isVariableDeclaration(parent.parent) &&
///     parent.parent.name.getText().includes(text.slice(1));
/// ```
///
/// So `const state = $state(0)` skips (name `state` contains `state`)
/// and `let { props } = $props()` skips (pattern text contains
/// `props`), but `const doubled = $state.count * 2` in a component
/// that declares `const state = writable(…)` is a store subscription
/// of `state`, exactly like upstream.
///
/// `props_id_is_rune` additionally mirrors upstream's `isPropsId` /
/// `isPropsDeclarationRune` pair: when some variable declaration binds
/// an identifier named `props` from an initializer that is exactly
/// `$props()`, every `$props.id()` occurrence is the rune's id getter
/// and is filtered from store resolution
/// (processInstanceScriptContent.ts:356-359).
pub fn collect_rune_scan_context(
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
) -> RuneScanContext {
    let mut ctx = RuneScanContext::default();
    let on_decl = |ctx: &mut RuneScanContext, decl: &VariableDeclaration<'_>| {
        for declarator in &decl.declarations {
            let Some(Expression::CallExpression(call)) = declarator.init.as_ref() else {
                continue;
            };
            let id_span = declarator.id.span();
            let Some(id_text) = source.get(id_span.start as usize..id_span.end as usize) else {
                continue;
            };
            // Upstream compares the initializer's exact source text
            // against the literal string `$props()` — same here.
            let init_span = call.span;
            if source.get(init_span.start as usize..init_span.end as usize) == Some("$props()")
                && pattern_mentions_props(&declarator.id)
            {
                ctx.props_id_is_rune = true;
            }
            let check = |ctx: &mut RuneScanContext, expr: &Expression<'_>| {
                let Expression::Identifier(ident) = expr else {
                    return;
                };
                let name = ident.name.as_str();
                if CONDITIONAL_RUNE_NAMES.contains(&name) && id_text.contains(&name[1..]) {
                    ctx.decl_offsets.insert(ident.span.start);
                }
            };
            check(&mut *ctx, &call.callee);
            for arg in &call.arguments {
                if let Some(expr) = arg.as_expression() {
                    check(&mut *ctx, expr);
                }
            }
        }
    };
    for stmt in &program.body {
        walk_statement_descend(stmt, &mut |node| match node {
            WalkNode::Statement(Statement::VariableDeclaration(decl)) => on_decl(&mut ctx, decl),
            WalkNode::Statement(Statement::ExportDeclaration(ed)) => {
                if let Declaration::VariableDeclaration(decl) = &ed.declaration {
                    on_decl(&mut ctx, decl);
                }
            }
            WalkNode::ForInitVarDecl(decl) => on_decl(&mut ctx, decl),
            WalkNode::Statement(_) => {}
        });
    }
    ctx
}

/// Rune-position context for the `$<ident>` scanner — see
/// [`collect_rune_scan_context`].
#[derive(Debug, Default)]
pub struct RuneScanContext {
    /// Byte offsets (of the `$`) of `$state` / `$derived` / `$props`
    /// identifiers in upstream's `is_rune` declaration position.
    pub decl_offsets: HashSet<u32>,
    /// True when a `props` binding is initialised from exactly
    /// `$props()` — makes every `$props.id()` occurrence rune usage.
    pub props_id_is_rune: bool,
}

/// Does the binding pattern mention an identifier named `props`, as a
/// bound name or an object-pattern key? Mirrors upstream's
/// `isPropsDeclarationRune` trigger — `handleIdentifier` fires for any
/// identifier named `props` visited inside the `$props()`-initialised
/// variable declaration, which includes destructure keys and rest
/// elements.
fn pattern_mentions_props(pat: &BindingPattern<'_>) -> bool {
    use oxc_ast::ast::PropertyKey;
    match pat {
        BindingPattern::BindingIdentifier(id) => id.name == "props",
        BindingPattern::ObjectPattern(obj) => {
            obj.properties.iter().any(|p| {
                matches!(&p.key, PropertyKey::StaticIdentifier(k) if k.name == "props")
                    || pattern_mentions_props(&p.value)
            }) || obj
                .rest
                .as_ref()
                .is_some_and(|r| pattern_mentions_props(&r.argument))
        }
        BindingPattern::ArrayPattern(arr) => {
            arr.elements
                .iter()
                .flatten()
                .any(|el| pattern_mentions_props(el))
                || arr
                    .rest
                    .as_ref()
                    .is_some_and(|r| pattern_mentions_props(&r.argument))
        }
        BindingPattern::AssignmentPattern(asn) => pattern_mentions_props(&asn.left),
    }
}

/// True when the program has a named import binding `derived` from
/// `'svelte/store'`. Mirrors upstream
/// `ImplicitStoreValues.isSvelteStoreDerivedImport`, which suppresses
/// the store-subscription declaration for exactly that import in
/// Svelte 5+ so `$derived(...)` keeps resolving to the rune.
pub fn has_svelte_store_derived_import(program: &oxc_ast::ast::Program<'_>) -> bool {
    program.body.iter().any(|stmt| {
        let Statement::ImportDeclaration(decl) = stmt else {
            return false;
        };
        if decl.source.value != "svelte/store" {
            return false;
        }
        decl.specifiers.iter().flatten().any(|spec| {
            matches!(
                spec,
                ImportDeclarationSpecifier::ImportSpecifier(s) if s.local.name == "derived"
            )
        })
    })
}

/// Like [`find_store_refs`] but accepts a pre-computed binding set,
/// letting callers union module-script and instance-script bindings
/// (a `$store` reference in instance can resolve to a binding declared
/// in `<script module>`).
///
/// `runes` carries the byte offsets (of the `$`) of `$state` /
/// `$derived` / `$props` occurrences that are genuine rune usage per
/// upstream's contextual gate — see [`collect_rune_scan_context`].
/// Occurrences NOT in the set are ordinary candidates: a Svelte-4
/// component with `const state = writable(…)` gets its `$state`
/// subscription like any other store.
pub fn find_store_refs_with_bindings(
    program: &Program<'_>,
    source: &str,
    bound: &HashSet<String>,
    runes: &RuneScanContext,
) -> Vec<SmolStr> {
    if bound.is_empty() {
        return Vec::new();
    }
    let mut probe = StoreRefProbe {
        source,
        bound,
        runes,
        seen: HashSet::new(),
        out: Vec::new(),
    };
    probe.visit_program(program);
    probe.out
}

/// Store subscriptions written in the template: every `$<ident>`
/// identifier reference in a template expression whose `<ident>` is a
/// script binding. Each expression range is parsed with oxc (a
/// declaration tag's payload as a declarator list), so object keys,
/// member names, regex bodies and string contents never count.
/// Mirrors upstream `svelte2tsx/nodes/Stores.ts`, which walks the
/// template AST for the same identifiers.
pub fn find_template_store_refs(
    fragment: &svn_parser::Fragment,
    source: &str,
    bound: &HashSet<String>,
) -> Vec<SmolStr> {
    if bound.is_empty() {
        return Vec::new();
    }
    let alloc = oxc_allocator::Allocator::default();
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    // `use:$action`, `transition:$fly`: the directive name itself is
    // the store reference (upstream `Stores.handleDirective`).
    for name in crate::template_directive_names(fragment) {
        if let Some(base) = name.strip_prefix('$')
            && bound.contains(base)
            && seen.insert(name.clone())
        {
            out.push(name);
        }
    }
    for expr in crate::template_expression_ranges(fragment) {
        let Some(text) = source.get(expr.range.start as usize..expr.range.end as usize) else {
            continue;
        };
        if !text.contains('$') {
            continue;
        }
        let wrapped = if expr.is_declaration {
            format!("let {text}\n;")
        } else {
            format!("({text}\n);")
        };
        let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
        let runes = collect_rune_scan_context(&parsed.program, &wrapped);
        let mut probe = StoreRefProbe {
            source: &wrapped,
            bound,
            runes: &runes,
            seen: HashSet::new(),
            out: Vec::new(),
        };
        probe.visit_program(&parsed.program);
        for name in probe.out {
            if seen.insert(name.clone()) {
                out.push(name);
            }
        }
    }
    out
}

/// AST walk behind [`find_store_refs_with_bindings`]: every
/// `$<ident>` identifier reference outside type positions.
struct StoreRefProbe<'b> {
    source: &'b str,
    bound: &'b HashSet<String>,
    runes: &'b RuneScanContext,
    seen: HashSet<SmolStr>,
    out: Vec<SmolStr>,
}

impl<'a> Visit<'a> for StoreRefProbe<'_> {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        let full = it.name.as_str();
        let Some(ident) = full.strip_prefix('$') else {
            return;
        };
        if ident.is_empty() || self.runes.decl_offsets.contains(&it.span.start) {
            return;
        }
        // Upstream's `isPropsId` filter: with a `props = $props()`
        // declaration in the file, `$props.id()` is the rune's id
        // getter, never a store subscription. Upstream matches the
        // exact member text `$props.id` (getText() comparison — no
        // trivia between the tokens) on a zero-argument call.
        if full == "$props"
            && self.runes.props_id_is_rune
            && followed_by_id_call(self.source.as_bytes(), it.span.end as usize)
        {
            return;
        }
        if self.bound.contains(ident) && self.seen.insert(SmolStr::from(full)) {
            self.out.push(SmolStr::from(full));
        }
    }

    // A type reference's name is a type, never a store (upstream skips
    // `TypeReferenceNode`); its type arguments and a `typeof $x` type
    // query still count.
    fn visit_ts_type_reference(&mut self, it: &oxc_ast::ast::TSTypeReference<'a>) {
        if let Some(args) = &it.type_arguments {
            self.visit_ts_type_parameter_instantiation(args);
        }
    }
}

/// True when the bytes at `pos` spell `.id()` (member name exactly
/// `id`, then a zero-argument call; ASCII whitespace allowed around
/// the parens, none inside the member — mirroring upstream's
/// `parent.getText() === '$props.id'`, which fails on any trivia
/// between `$props`, `.`, and `id`).
fn followed_by_id_call(bytes: &[u8], pos: usize) -> bool {
    let Some(rest) = bytes.get(pos..) else {
        return false;
    };
    let Some(after_id) = rest.strip_prefix(b".id") else {
        return false;
    };
    // Member name must be exactly `id` — reject `.identifier`.
    if let Some(&next) = after_id.first()
        && (next.is_ascii_alphanumeric() || next == b'_' || next == b'$' || next >= 0x80)
    {
        return false;
    }
    let mut k = 0;
    while after_id.get(k).is_some_and(u8::is_ascii_whitespace) {
        k += 1;
    }
    if after_id.get(k) != Some(&b'(') {
        return false;
    }
    k += 1;
    while after_id.get(k).is_some_and(u8::is_ascii_whitespace) {
        k += 1;
    }
    after_id.get(k) == Some(&b')')
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
        Statement::ExportDeclaration(decl) => collect_from_declaration(&decl.declaration, out),
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
    fn rune_decl_positions_excluded() {
        // `const state = $state(0)` etc. — the rune callee is the init
        // of a declarator whose name contains the rune base, so it is
        // NOT a store subscription (upstream's `is_rune` gate in
        // processInstanceScriptContent.ts). The template's `{state}`
        // refs are plain identifiers and irrelevant here.
        let src =
            "let { props } = $props();\nlet state = $state(0);\nlet derived = $derived(state * 2);";
        assert!(refs(src).is_empty());
    }

    #[test]
    fn store_named_like_rune_subscribes_outside_rune_decl() {
        // A Svelte-4 store literally named `state`: `$state.count` is a
        // subscription to it, not rune usage — the occurrence is not a
        // call-init of a related declarator. Upstream emits
        // `let $state = __sveltets_2_store_get(state);` here.
        let src = "const state = writable({ count: 0 });\nconst doubled = $state.count * 2;";
        assert_eq!(refs(src), vec!["$state"]);
    }

    #[test]
    fn bare_rune_lookalike_refs_subscribe() {
        // Upstream's stores-looking-like-runes sample: bare (non-call)
        // `$props` / `$state` / `$derived` references with same-named
        // consts all become store subscriptions.
        let src = "const props = null;\n$props;\nconst state = null;\n$state;\nconst derived = null;\n$derived;";
        assert_eq!(refs(src), vec!["$props", "$state", "$derived"]);
    }

    #[test]
    fn unrelated_decl_name_keeps_rune_lookalike_as_store() {
        // `const x = $state(0)` — the declarator name `x` does not
        // contain `state`, so upstream does NOT treat the occurrence as
        // a rune; with a top-level `state` binding it subscribes.
        let src = "let state = null;\nconst x = $state(0);";
        assert_eq!(refs(src), vec!["$state"]);
    }

    #[test]
    fn effect_named_binding_subscribes() {
        // Upstream never special-cases `$effect` / `$bindable` /
        // `$inspect` / `$host` in store resolution — a binding named
        // `effect` makes `$effect` a store subscription.
        let src = "let effect = null;\nconst x = $effect;";
        assert_eq!(refs(src), vec!["$effect"]);
    }

    #[test]
    fn props_id_is_rune_when_props_declared_from_props_rune() {
        // Upstream's isPropsId / isPropsDeclarationRune pair: with a
        // `props` binding initialised from exactly `$props()`,
        // `$props.id()` is the rune's id getter — no subscription.
        for decl in [
            "let props = $props();",
            "let { props } = $props();",
            "let {...props} = $props();",
        ] {
            let src = format!("{decl}\nlet id = $props.id();");
            assert!(refs(&src).is_empty(), "expected no refs for: {decl}");
        }
    }

    #[test]
    fn props_id_subscribes_when_props_not_rune_initialised() {
        // `let props = {};` — no $props() initialiser, so `$props.id()`
        // resolves as a store subscription of `props`. Upstream emits
        // `let $props = __sveltets_2_store_get(props);` here.
        let src = "let props = {};\nlet id = $props.id();";
        assert_eq!(refs(src), vec!["$props"]);
    }

    #[test]
    fn svelte_store_derived_import_never_subscribes() {
        // `import { derived } from 'svelte/store'` + `$derived(...)` —
        // mirrors upstream ImplicitStoreValues.isSvelteStoreDerivedImport
        // (Svelte 5+): no subscription, `$derived` stays the rune.
        let src = "import { derived } from 'svelte/store';\nlet a = $derived(1);";
        assert!(refs(src).is_empty());
        // A `derived` import from anywhere else DOES subscribe.
        let src2 = "import { derived } from './stores';\nlet a = $derived(1);";
        assert_eq!(refs(src2), vec!["$derived"]);
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
    fn unicode_store_name_recognized() {
        // `$café` must read the full Unicode name and match the binding —
        // an ASCII-only scan would truncate to `$caf` and miss it.
        let src = "let café = null;\nconst x = $café;";
        assert!(refs(src).iter().any(|s| s == "$café"));
        let src2 = "let 日本語 = null;\nconst y = $日本語;";
        assert!(refs(src2).iter().any(|s| s == "$日本語"));
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
