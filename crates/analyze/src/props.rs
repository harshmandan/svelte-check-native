//! Props analysis — central source of truth for the component's Props
//! bag.
//!
//! Mirrors upstream's
//! `svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts` structurally:
//! one analyze-time pass produces every decision emit later makes
//! about the component's Props. The result lives on [`PropsInfo`],
//! built once by [`PropsInfo::build`] and consumed read-only by emit.
//!
//! ### What's in PropsInfo
//!
//! - `source` — where the Props type text came from
//!   ([`PropsSource`]). Drives decisions like SvelteKit route auto-
//!   typing (only fires when `source == PropsSource::None`).
//! - `type_text` — the raw Props type as a source slice or
//!   synthesised string. `None` when no annotation / no `$$Props` /
//!   no `export let` was found.
//! - `type_root_name` — the leading named-type reference in
//!   `type_text`, if any. Script-split's hoisting decision reads this.
//! - `destructures` — every local introduced by `let { ... } =
//!   $props()` at top-level, in source order. Feeds `void <name>;`
//!   emission so `noUnusedLocals` doesn't fire on props only consumed
//!   via `bind:` / spread / template.
//!
//! ### Destructuring patterns handled
//!
//! - `let { foo } = $props()`                          → local = `foo`
//! - `let { foo = defaultVal } = $props()`             → local = `foo`
//! - `let { class: classValue } = $props()`            → local = `classValue`
//! - `let { foo, ...rest } = $props()`                 → locals = `foo`, `rest`
//! - `let { foo }: FooProps = $props()`                → local = `foo`
//! - `let { foo } = $props<Props>()`                   → local = `foo`
//!
//! Nested destructuring (`let { foo: { bar } } = $props()`) is walked
//! recursively; every leaf identifier is recorded.
//!
//! ### Type-source priority
//!
//! First match wins:
//!
//! 1. `let { ... }: PropType = $props()` → PropType source slice.
//! 2. `let { ... } = $props<PropType>()` → PropType source slice.
//! 3. `interface $$Props { ... }` at module scope → the literal string
//!    `"$$Props"`. The interface declaration itself hoists via
//!    script_split alongside other user interfaces.
//! 4. `export let foo: T; export let bar = 42;` (Svelte-4) →
//!    synthesize `{ foo: T; bar?: any; ... }` from each top-level
//!    `export let`/`export const` declaration (plus `export { alias }`
//!    specifiers).
//! 5. None of the above → `PropsSource::None`; emit falls back to
//!    `any`.

use oxc_ast::ast::{
    BindingPattern, BindingProperty, Declaration, Expression, ModuleExportName, PropertyKey,
    Statement, VariableDeclaration,
};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_core::Range;

/// One destructured prop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PropInfo {
    /// The local name introduced by the destructuring — what later code
    /// in the script refers to. For a rename `{ class: classValue }` this
    /// is `classValue`, not `class`.
    pub local_name: SmolStr,
    /// The original prop key — what the parent passes when instantiating
    /// the component. For rename `{ class: classValue }` this is `class`;
    /// for shorthand `{ foo }` this equals `local_name`.
    pub prop_key: SmolStr,
    /// Byte range of the local identifier in the source.
    pub range: Range,
    /// True for `...rest` elements.
    pub is_rest: bool,
    /// True when the destructure entry has a default value (`= …`)
    /// or is wrapped in `$bindable(...)`. JS-overlay Props synthesis
    /// uses this to mark the prop as optional in the typedef.
    pub has_default: bool,
    /// True when the destructure entry's default is `$bindable(...)`
    /// — the Svelte 5 marker that the prop participates in two-way
    /// binding. Excluded from the divergence check that decides
    /// whether to synthesise `$$ComponentProps` vs use the user's
    /// Props typedef: bindable-only extras like `element = $bindable()`
    /// are typically not declared in Props by convention, but
    /// upstream svelte2tsx still keeps Props (the destructure is
    /// effectively a subset of "real" props).
    pub is_bindable: bool,
    /// Type text inferred from the default-value literal — used by
    /// the JS-overlay `$$ComponentProps` synthesis when the prop has
    /// a default. `None` when no default is present, or when the
    /// default's expression doesn't match a recognised literal form
    /// (caller falls back to `any`).
    ///
    /// Mirrors upstream svelte2tsx's per-default inference:
    ///   `= ''`            → "string"
    ///   `= 0`             → "number"
    ///   `= true|false`    → "boolean"
    ///   `= null`          → "null"
    ///   `= {}`            → "Record<string, any>"
    ///   `= []`            → "any[]"
    ///   `= () => {…}`     → "Function"
    ///   `= function(){…}` → "Function"
    ///   `= $bindable(x)`  → recurse on `x`
    ///   anything else     → `None`
    pub default_type_text: Option<SmolStr>,
}

/// Where the Props type text in [`PropsInfo::type_text`] came from.
///
/// Downstream emit branches on this rather than re-deriving the shape
/// from `type_text`. Keeps the "how did we get here" decision in one
/// place and makes it trivial to test the chosen branch per fixture.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum PropsSource {
    /// No props-shape detected at all. `type_text` is `None`.
    /// Emit may still synthesise a SvelteKit route shape before
    /// falling back to `any`.
    #[default]
    None,
    /// `let { ... }: PropType = $props()` — annotation on the
    /// destructure binding.
    RuneAnnotation,
    /// `let { ... } = $props<PropType>()` — generic arg on the call.
    RuneGeneric,
    /// Svelte-4 `interface $$Props { ... }` at module scope.
    LegacyInterface,
    /// Synthesised from `export let` / `export const` declarations
    /// and `export { local as alias }` specifiers.
    SynthesisedFromExports,
    /// Synthesised `{ k: T, k?: U, … }` literal from the `$props()`
    /// destructure defaults — mirrors upstream svelte2tsx's "Hard mode"
    /// best-effort synthesis for TS-source files with no annotation.
    /// Populated post-hoc by emit (not `PropsInfo::build`) because it
    /// depends on the source script's lang, which analyze does not
    /// consult. See `synthesise_destructure_props_literal` in emit.
    SynthesisedFromDestructure,
}

/// Everything emit needs to know about a component's Props, resolved
/// once by [`PropsInfo::build`]. Replaces the scattered
/// `find_props_type_source` / `find_props` / `root_type_name` calls
/// the emit hot path used to make.
#[derive(Debug, Clone, Default)]
pub struct PropsInfo {
    /// Where the Props type text came from.
    pub source: PropsSource,
    /// Raw Props type text — a source slice for rune/$$Props shapes,
    /// or a freshly-synthesised object type for Svelte-4 export-let.
    /// `None` when `source == PropsSource::None`.
    pub type_text: Option<String>,
    /// Leading named-type reference in `type_text`, if any. Populated
    /// only when `type_text` starts with an identifier-ish token
    /// (e.g. `Props`, `Props<T>`, `ChannelMessageProps`). `None` for
    /// literal shapes (`{ ... }`), tuples, unions, intersections, or
    /// when `type_text` is `None`.
    pub type_root_name: Option<SmolStr>,
    /// Every local name introduced by `let { ... } = $props()` at
    /// top-level, in source order. Empty for Svelte-4 components and
    /// for components with no `$props()` call.
    pub destructures: Vec<PropInfo>,
}

impl PropsInfo {
    /// Build [`PropsInfo`] from a parsed instance-script program.
    ///
    /// Single pass through `program.body`: walks variable
    /// declarations for a `$props()` call (Shape 1/2), scans for an
    /// `interface $$Props` at module scope (Shape 3), and — only
    /// when neither is found — walks top-level `export let` /
    /// `export const` / `export { alias }` to synthesise a Svelte-4
    /// Props shape (Shape 4).
    ///
    /// `program` MUST be the instance-script program. Module-script
    /// `export let`s are module-scope exports, not component props.
    pub fn build(program: &oxc_ast::ast::Program<'_>, source: &str) -> Self {
        let mut destructures: Vec<PropInfo> = Vec::new();
        let mut type_text: Option<String> = None;
        let mut props_source = PropsSource::None;

        // Shape 1 / Shape 2: explicit `$props()` annotation wins over
        // everything else. Collect the destructured names from the
        // same call while we're here.
        for stmt in &program.body {
            let Statement::VariableDeclaration(decl) = stmt else {
                continue;
            };
            for declarator in &decl.declarations {
                let Some(init) = declarator.init.as_ref() else {
                    continue;
                };
                if !is_props_call_like(init) {
                    continue;
                }
                collect_from_binding(&declarator.id, &mut destructures);
                if type_text.is_some() {
                    continue;
                }
                if let Some(ty) = declarator.type_annotation.as_ref() {
                    let span = ty.type_annotation.span();
                    if let Some(slice) = source.get(span.start as usize..span.end as usize) {
                        type_text = Some(slice.to_string());
                        props_source = PropsSource::RuneAnnotation;
                        continue;
                    }
                }
                if let Expression::CallExpression(call) = init
                    && let Some(tp) = call.type_arguments.as_ref()
                    && let Some(arg) = tp.params.first()
                {
                    let span = arg.span();
                    if let Some(slice) = source.get(span.start as usize..span.end as usize) {
                        type_text = Some(slice.to_string());
                        props_source = PropsSource::RuneGeneric;
                    }
                }
            }
        }

        if type_text.is_none() {
            // Shape 3: Svelte-4 `interface $$Props { ... }`.
            for stmt in &program.body {
                if let Statement::TSInterfaceDeclaration(iface) = stmt
                    && iface.id.name == "$$Props"
                {
                    type_text = Some("$$Props".to_string());
                    props_source = PropsSource::LegacyInterface;
                    break;
                }
            }
        }

        if type_text.is_none()
            && let Some(synth) = synthesize_props_type_from_export_let(program, source)
        {
            // Shape 4: export-let fallback.
            type_text = Some(synth);
            props_source = PropsSource::SynthesisedFromExports;
        }

        let type_root_name = type_text.as_deref().and_then(root_type_name_of);

        Self {
            source: props_source,
            type_text,
            type_root_name,
            destructures,
        }
    }
}

/// Compute the leading named-type reference of `ty`, if any. Returns
/// `None` for literal shapes (`{ ... }`), tuple/array (`[...]`), and
/// other non-reference starts. Used by script_split's hoisting
/// decision: a named-type Props can be mentioned at module scope
/// (script_split hoists its declaration); a literal shape stays
/// inline.
pub fn root_type_name_of(ty: &str) -> Option<SmolStr> {
    let ty = ty.trim_start();
    let bytes = ty.as_bytes();
    if bytes.is_empty() {
        return None;
    }
    let first = bytes[0];
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return None;
    }
    let mut end = 0usize;
    while end < bytes.len()
        && (bytes[end].is_ascii_alphanumeric() || bytes[end] == b'_' || bytes[end] == b'$')
    {
        end += 1;
    }
    Some(SmolStr::from(&ty[..end]))
}

/// Build an inline object type literal from top-level `export let` /
/// `export const` declarations in an instance script. Returns `None`
/// when there are no such declarations.
///
/// `program` here is the instance script AST. Module-script `export
/// let`s are NOT component props — those are module-scope exports.
/// Callers must pass the instance-script program, not the module
/// script's.
///
/// Each declarator becomes one property on the synthesized type:
///
/// - `export let foo: T;` → `foo: T;` (required)
/// - `export let foo: T = v;` → `foo?: T;` (optional — has default)
/// - `export let foo = v;` → `foo?: any;` (no type; inference is
///   nontrivial without tsgo, so we pick `any` over mislabeling)
/// - `export let foo;` → `foo: any;` (no type, no default)
/// - `export const foo: T = v;` → `foo?: T;` (has initializer; `const`
///   always has one, so always optional for defaulting)
///
/// Non-identifier patterns (e.g. `export let { a } = obj;`) are skipped:
/// destructured exports are not valid Svelte prop declarations.
fn synthesize_props_type_from_export_let(
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for stmt in &program.body {
        let Statement::ExportNamedDeclaration(export_decl) = stmt else {
            continue;
        };
        if let Some(Declaration::VariableDeclaration(var_decl)) = &export_decl.declaration {
            append_props_from_var_decl(var_decl, source, &mut parts);
        }
        // `export { name as alias, ... }` specifier form. Svelte 4
        // components use this to expose a local under a JS reserved
        // name, most commonly `export { className as class }` so
        // consumers can pass `class={...}` without hitting `class`
        // being a keyword in the source. Each specifier contributes
        // one prop; the local's declared type (if any) is preserved.
        for spec in &export_decl.specifiers {
            append_prop_from_export_specifier(spec, program, source, &mut parts);
        }
    }
    if parts.is_empty() {
        return None;
    }
    let mut out = String::with_capacity(parts.iter().map(|p| p.len() + 2).sum::<usize>() + 4);
    out.push_str("{ ");
    for (i, part) in parts.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        out.push_str(part);
    }
    out.push_str(" }");
    Some(out)
}

/// Extract a single property from `export { local as alias }`. Looks up
/// `local` in the program's top-level `let`/`const`/`var` declarations
/// to pick up its type annotation and initializer (which decides the
/// optional marker). When `local` can't be found or has no annotation,
/// the prop is typed `any`. The alias becomes the public key so
/// consumers write `<Foo {alias}=...>`.
fn append_prop_from_export_specifier(
    spec: &oxc_ast::ast::ExportSpecifier<'_>,
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
    out: &mut Vec<String>,
) {
    // Re-export `export { X } from 'mod'` (source specifier on the
    // parent statement) and namespace re-exports don't declare a
    // component prop — the parent's `source` field catches re-exports;
    // here we only care about local-to-alias renames.
    let Some(alias) = module_export_name_str(&spec.exported) else {
        return;
    };
    let Some(local) = module_export_name_str(&spec.local) else {
        return;
    };
    let (ty_text, has_init) = find_local_type_and_init(program, source, local);
    let optional_marker = if has_init { "?" } else { "" };
    let ty_src = ty_text.unwrap_or("any");
    out.push(format!("{alias}{optional_marker}: {ty_src};"));
}

/// Readable `str` from a `ModuleExportName` variant. Returns `None` for
/// `StringLiteral` — a string-literal alias like `export { foo as 'a-b' }`
/// isn't usable as an object-type key here (we'd have to quote it, and
/// the Svelte idiom doesn't produce them).
fn module_export_name_str<'a>(name: &'a ModuleExportName<'_>) -> Option<&'a str> {
    match name {
        ModuleExportName::IdentifierName(id) => Some(id.name.as_str()),
        ModuleExportName::IdentifierReference(id) => Some(id.name.as_str()),
        ModuleExportName::StringLiteral(_) => None,
    }
}

/// Walk the program's top-level `let`/`const`/`var` declarations looking
/// for one that binds `name`. Returns `(type_text, has_init)` — the type
/// annotation's source slice when present, and whether the declarator
/// has an initializer (so the caller can mark the prop optional vs
/// required the same way `append_props_from_var_decl` does for the
/// `export let` form).
fn find_local_type_and_init<'s>(
    program: &oxc_ast::ast::Program<'_>,
    source: &'s str,
    name: &str,
) -> (Option<&'s str>, bool) {
    for stmt in &program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        for declarator in &decl.declarations {
            let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                continue;
            };
            if id.name.as_str() != name {
                continue;
            }
            let ty_text = declarator
                .type_annotation
                .as_ref()
                .map(|ty| ty.type_annotation.span())
                .and_then(|span| source.get(span.start as usize..span.end as usize));
            return (ty_text, declarator.init.is_some());
        }
    }
    (None, false)
}

fn append_props_from_var_decl(
    var_decl: &VariableDeclaration<'_>,
    source: &str,
    out: &mut Vec<String>,
) {
    for declarator in &var_decl.declarations {
        let BindingPattern::BindingIdentifier(id) = &declarator.id else {
            // Destructured exports aren't valid Svelte prop declarations;
            // skip them rather than invent a synthetic name.
            continue;
        };
        let name = id.name.as_str();
        let has_init = declarator.init.is_some();
        let ty_text: Option<&str> = declarator
            .type_annotation
            .as_ref()
            .map(|ty| ty.type_annotation.span())
            .and_then(|span| source.get(span.start as usize..span.end as usize));
        // An initializer makes the prop optional (caller can omit and
        // the default kicks in). No initializer + no type → required
        // `any`. No initializer + type → required of that type.
        let optional_marker = if has_init { "?" } else { "" };
        // Inference fallback when there's no explicit annotation.
        // For `export let fn = (x: T, y: U) => …` we synthesize the
        // arrow's parameter signature — this is what lets a consumer
        // passing `<Comp fn={cond ? (x, y) => … : alt} />` contextually
        // type its arrow's params against the component prop. Without
        // the signature, the prop collapses to `any` and every
        // consumer's callback param fires TS7006 implicit-any.
        let arrow_sig = if ty_text.is_none() {
            declarator
                .init
                .as_ref()
                .and_then(|init| arrow_signature_from_init(init, source))
        } else {
            None
        };
        let ty_src = ty_text
            .map(ToOwned::to_owned)
            .or(arrow_sig)
            // Unannotated with an initializer: `typeof <name>` lets
            // TS pick up the initializer-inferred type (e.g.
            // `let translate = writable({x,y})` →
            // `Writable<{x,y}>`) instead of collapsing to `any`.
            // Mirrors upstream svelte2tsx's ExportedNames emit for
            // Svelte-4 prop types (`createReturnElementsType` →
            // `${key}?: typeof ${key}`).
            //
            // Unannotated with NO initializer: `typeof <name>` at
            // a `let <name>;` site narrows to `undefined` under
            // strict mode, rejecting consumer writes. Fall back to
            // `any` for that case — matches the prior behavior for
            // legitimately-uninitialised props.
            //
            // Critical: the `typeof <name>` form is embedded inside
            // `$$render`'s body scope where it resolves. Any
            // module-scope consumer of this type MUST route through
            // the `Awaited<ReturnType<typeof $$render>>['props']`
            // projection instead of naming the literal directly —
            // `contains_typeof_ref` below flags that.
            .unwrap_or_else(|| {
                if has_init {
                    format!("typeof {name}")
                } else {
                    "any".to_string()
                }
            });
        out.push(format!("{name}{optional_marker}: {ty_src};"));
    }
}

/// Cheap substring check: does a Props type-source string reference
/// any body-local via `typeof <name>`? Used by emit to decide whether
/// the literal is safe to name at module scope or must go through the
/// `Awaited<ReturnType<typeof $$render>>['props']` projection.
pub fn contains_typeof_ref(ty: &str) -> bool {
    // Match `typeof ` (with trailing space) — the regular TS grammar
    // form. Comments / string literals with the substring are not a
    // real concern because this synthesis never embeds user comments
    // into the output; it only concatenates structured type text.
    ty.contains("typeof ")
}

/// For `init` = an arrow function, synthesize a function-type
/// annotation from its parameter signatures. Return type defaults
/// to `any` — we don't try to infer it without running TS. Returns
/// `None` if the init isn't an arrow, or any param uses a pattern
/// we don't emit (destructure, rest) — better to fall back to `any`
/// than emit an incomplete signature that tsgo rejects.
fn arrow_signature_from_init(init: &Expression<'_>, source: &str) -> Option<String> {
    let Expression::ArrowFunctionExpression(arrow) = init else {
        return None;
    };
    let mut parts: Vec<String> = Vec::new();
    for param in &arrow.params.items {
        let BindingPattern::BindingIdentifier(id) = &param.pattern else {
            // Destructure / rest / assignment patterns — give up and
            // let the caller fall back to `any`. Writing these as
            // prop-type-position types needs the full TS
            // destructure-type machinery we don't reproduce.
            return None;
        };
        let name = id.name.as_str();
        let ty = param
            .type_annotation
            .as_ref()
            .map(|ty| ty.type_annotation.span())
            .and_then(|span| source.get(span.start as usize..span.end as usize))
            .unwrap_or("any");
        parts.push(format!("{name}: {ty}"));
    }
    // Return type — honor an explicit annotation, otherwise `any`.
    // We don't run an inference pass; `any` is strictly looser than
    // the real return, which is safe for prop-sig contextual typing.
    let ret = arrow
        .return_type
        .as_ref()
        .map(|r| r.type_annotation.span())
        .and_then(|span| source.get(span.start as usize..span.end as usize))
        .unwrap_or("any");
    Some(format!("({}) => {}", parts.join(", "), ret))
}

/// Does this expression look like a call to the `$props` rune?
///
/// Matches `$props()`, `$props<Type>()`, `$props<{...}>()`. Doesn't match
/// dotted variants (`$props.id()` — that's a different rune).
fn is_props_call_like(expr: &Expression<'_>) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    matches!(&call.callee, Expression::Identifier(id) if id.name == "$props")
}

/// SVELTE-4-COMPAT — typed-events narrowing source.
///
/// Find the first top-level `createEventDispatcher<T>()` call in
/// `program` and return the source slice of `T`. The caller splices it
/// into a synthesised `type $$Events = <T>;` so Item 3's existing
/// intersection narrows child event handlers.
///
/// Simple-identifier callee only (`createEventDispatcher`); import
/// aliasing (`import { createEventDispatcher as d }`) is not tracked —
/// missing that case falls through to the lax shim which is correct,
/// just non-optimal.
///
/// Returns the FIRST hit's type arg. Multi-declarator statements
/// (`const a = dispatcher<A>(), b = dispatcher<B>()`) and multiple
/// dispatcher calls in one file aren't supported; `$$Events` only has
/// one declaration slot and the common pattern is one dispatcher per
/// component.
pub fn find_dispatcher_event_type_source(
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
) -> Option<String> {
    for stmt in &program.body {
        let maybe_slice = match stmt {
            Statement::VariableDeclaration(decl) => decl
                .declarations
                .iter()
                .filter_map(|d| d.init.as_ref())
                .find_map(|e| dispatcher_type_arg_slice(e, source)),
            Statement::ExpressionStatement(expr_stmt) => {
                dispatcher_type_arg_slice(&expr_stmt.expression, source)
            }
            _ => None,
        };
        if let Some(slice) = maybe_slice {
            return Some(slice);
        }
    }
    None
}

/// Find local names bound to a `createEventDispatcher(...)` call at
/// top level: `const NAME = createEventDispatcher(...)` (any
/// type-arg form). Used by [`find_dispatched_event_names`] to
/// scope the event-name scan to actual dispatcher calls.
///
/// Aliased imports (`import { createEventDispatcher as d }`) aren't
/// resolved — falling through to no-events is safe (the user opts
/// in via `interface $$Events` or runes mode for those cases).
pub fn find_dispatcher_local_names(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for stmt in &program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        for d in &decl.declarations {
            let Some(init) = &d.init else { continue };
            let Expression::CallExpression(call) = init else {
                continue;
            };
            let Expression::Identifier(id) = &call.callee else {
                continue;
            };
            if id.name != "createEventDispatcher" {
                continue;
            }
            let BindingPattern::BindingIdentifier(bid) = &d.id else {
                continue;
            };
            out.push(bid.name.to_string());
        }
    }
    out
}

/// Scan `program` for `<dispatcher>(<string-literal>, ...)` calls
/// where `<dispatcher>` is one of `dispatcher_locals`. Returns the
/// union of distinct event-name string literals in source order.
///
/// Used by the untyped-dispatcher synth path
/// (`<script strictEvents>` or runes mode without `interface
/// $$Events` and without an explicit `createEventDispatcher<T>()`
/// type arg). Each name flows into a synthesized `type $$Events =
/// { name1: any, name2: any, … }` so the consumer-side `$on('name',
/// cb)` resolves cb to `(e: any) => any` — narrowed from "any
/// string" to the actual dispatched-name set.
pub fn find_dispatched_event_names(
    program: &oxc_ast::ast::Program<'_>,
    dispatcher_locals: &[String],
) -> Vec<String> {
    use std::collections::HashSet;
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for stmt in &program.body {
        scan_statement_for_dispatched_names(stmt, dispatcher_locals, &mut seen, &mut out);
    }
    out
}

fn scan_statement_for_dispatched_names(
    stmt: &Statement<'_>,
    dispatcher_locals: &[String],
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let Some(init) = &d.init {
                    scan_expression_for_dispatched_names(init, dispatcher_locals, seen, out);
                }
            }
        }
        Statement::ExpressionStatement(es) => {
            scan_expression_for_dispatched_names(&es.expression, dispatcher_locals, seen, out);
        }
        Statement::FunctionDeclaration(fd) => {
            if let Some(body) = &fd.body {
                for s in &body.statements {
                    scan_statement_for_dispatched_names(s, dispatcher_locals, seen, out);
                }
            }
        }
        Statement::ReturnStatement(rs) => {
            if let Some(arg) = &rs.argument {
                scan_expression_for_dispatched_names(arg, dispatcher_locals, seen, out);
            }
        }
        Statement::IfStatement(s) => {
            scan_expression_for_dispatched_names(&s.test, dispatcher_locals, seen, out);
            scan_statement_for_dispatched_names(&s.consequent, dispatcher_locals, seen, out);
            if let Some(alt) = &s.alternate {
                scan_statement_for_dispatched_names(alt, dispatcher_locals, seen, out);
            }
        }
        Statement::BlockStatement(b) => {
            for s in &b.body {
                scan_statement_for_dispatched_names(s, dispatcher_locals, seen, out);
            }
        }
        _ => {}
    }
}

fn scan_expression_for_dispatched_names(
    expr: &Expression<'_>,
    dispatcher_locals: &[String],
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    match expr {
        Expression::CallExpression(call) => {
            // Match `<local>(<string-literal>, ...)` where <local>
            // is a known dispatcher binding.
            if let Expression::Identifier(id) = &call.callee
                && dispatcher_locals.iter().any(|n| n.as_str() == id.name.as_str())
                && let Some(first) = call.arguments.first()
                && let Some(first_expr) = first.as_expression()
                && let Expression::StringLiteral(s) = first_expr
            {
                let name = s.value.to_string();
                if seen.insert(name.clone()) {
                    out.push(name);
                }
            }
            // Recurse into callee + args to catch nested calls
            // (e.g. `wrap(dispatch('foo', payload))`).
            scan_expression_for_dispatched_names(&call.callee, dispatcher_locals, seen, out);
            for a in &call.arguments {
                if let Some(e) = a.as_expression() {
                    scan_expression_for_dispatched_names(e, dispatcher_locals, seen, out);
                }
            }
        }
        Expression::ArrowFunctionExpression(arrow) => {
            for s in &arrow.body.statements {
                scan_statement_for_dispatched_names(s, dispatcher_locals, seen, out);
            }
        }
        Expression::FunctionExpression(fe) => {
            if let Some(body) = &fe.body {
                for s in &body.statements {
                    scan_statement_for_dispatched_names(s, dispatcher_locals, seen, out);
                }
            }
        }
        _ => {}
    }
}

/// AST-based check: does `program` contain at least one call to
/// `createEventDispatcher(...)` (typed or untyped, top-level or
/// nested in an initializer / function body)? Used by the default-
/// export shape decision to choose between the
/// `__sveltets_2_fn_component`-equivalent `Component<P, X, B>`
/// shape (no events) and the iso-component interface (events
/// present).
///
/// Substring detection on raw source text false-positives on
/// comments (`// uses createEventDispatcher`), string literals, and
/// unused imports — none of which actually emit events. The AST
/// walk only fires on real call expressions.
///
/// Aliased imports (`import { createEventDispatcher as d }`) aren't
/// resolved here — falling through to the iso shape is safe (one
/// extra indirection, no correctness loss). Future #3b slice will
/// thread the import alias map through.
pub fn has_event_dispatcher_call(program: &oxc_ast::ast::Program<'_>) -> bool {
    program.body.iter().any(statement_has_dispatcher_call)
}

fn statement_has_dispatcher_call(stmt: &Statement<'_>) -> bool {
    match stmt {
        Statement::VariableDeclaration(decl) => decl
            .declarations
            .iter()
            .filter_map(|d| d.init.as_ref())
            .any(expression_has_dispatcher_call),
        Statement::ExpressionStatement(es) => expression_has_dispatcher_call(&es.expression),
        Statement::FunctionDeclaration(fd) => fd
            .body
            .as_ref()
            .is_some_and(|body| body.statements.iter().any(statement_has_dispatcher_call)),
        Statement::IfStatement(s) => {
            statement_has_dispatcher_call(&s.consequent)
                || s.alternate
                    .as_ref()
                    .is_some_and(statement_has_dispatcher_call)
        }
        Statement::BlockStatement(b) => b.body.iter().any(statement_has_dispatcher_call),
        _ => false,
    }
}

fn expression_has_dispatcher_call(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::CallExpression(call) => {
            if let Expression::Identifier(id) = &call.callee
                && id.name == "createEventDispatcher"
            {
                return true;
            }
            if expression_has_dispatcher_call(&call.callee) {
                return true;
            }
            call.arguments.iter().any(|a| {
                a.as_expression()
                    .is_some_and(expression_has_dispatcher_call)
            })
        }
        Expression::ArrowFunctionExpression(arrow) => arrow
            .body
            .statements
            .iter()
            .any(statement_has_dispatcher_call),
        Expression::FunctionExpression(fe) => fe
            .body
            .as_ref()
            .is_some_and(|body| body.statements.iter().any(statement_has_dispatcher_call)),
        _ => false,
    }
}

/// If `expr` is a `createEventDispatcher<T>(...)` call with an explicit
/// type argument, return `T`'s source text. Otherwise `None`.
fn dispatcher_type_arg_slice(expr: &Expression<'_>, source: &str) -> Option<String> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    match &call.callee {
        Expression::Identifier(id) if id.name == "createEventDispatcher" => {}
        _ => return None,
    }
    let tp = call.type_arguments.as_ref()?;
    let arg = tp.params.first()?;
    let span = arg.span();
    source
        .get(span.start as usize..span.end as usize)
        .map(str::to_string)
}

fn collect_from_binding(pat: &BindingPattern<'_>, out: &mut Vec<PropInfo>) {
    match pat {
        BindingPattern::ObjectPattern(obj) => {
            for prop in &obj.properties {
                collect_from_object_property(prop, out);
            }
            if let Some(rest) = &obj.rest {
                collect_rest(&rest.argument, out, true);
            }
        }
        // `let [a, b, c] = $props()` isn't a valid Svelte pattern ($props
        // returns an object), but be defensive.
        BindingPattern::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_from_binding(el, out);
            }
        }
        BindingPattern::BindingIdentifier(id) => {
            let name = SmolStr::from(id.name.as_str());
            out.push(PropInfo {
                local_name: name.clone(),
                prop_key: name,
                range: Range::new(id.span.start, id.span.end),
                is_rest: false,
                has_default: false,
                is_bindable: false,
                default_type_text: None,
            });
        }
        BindingPattern::AssignmentPattern(asn) => {
            // `name = default` at the top level (no surrounding object
            // pattern key) — the local name is the LHS identifier and
            // `has_default` is true. Mostly hit through nested patterns;
            // top-level entries flow through `collect_from_object_property`
            // which sets has_default itself.
            let before = out.len();
            collect_from_binding(&asn.left, out);
            let inferred = infer_default_type(&asn.right);
            let bindable = is_bindable_call(&asn.right);
            for entry in &mut out[before..] {
                entry.has_default = true;
                if bindable {
                    entry.is_bindable = true;
                }
                if entry.default_type_text.is_none() {
                    entry.default_type_text = inferred.clone();
                }
            }
        }
    }
}

fn collect_from_object_property(prop: &BindingProperty<'_>, out: &mut Vec<PropInfo>) {
    // Shorthand `{ foo }` vs rename `{ foo: bar }` vs default-value
    // `{ foo = bar }` vs `{ foo: bar = baz }`. The local name lives in
    // `prop.value`; the prop key is `prop.key` (set when not shorthand);
    // a top-level AssignmentPattern in `prop.value` carries the default.
    let prop_key: Option<SmolStr> = match &prop.key {
        PropertyKey::StaticIdentifier(id) => Some(SmolStr::from(id.name.as_str())),
        PropertyKey::StringLiteral(s) => Some(SmolStr::from(s.value.as_str())),
        _ => None,
    };
    let before = out.len();
    let (has_default, inferred_default, bindable) = match &prop.value {
        BindingPattern::AssignmentPattern(asn) => (
            true,
            infer_default_type(&asn.right),
            is_bindable_call(&asn.right),
        ),
        _ => (false, None, false),
    };
    collect_from_binding(&prop.value, out);
    // Patch the entries this property added: their prop_key should
    // reflect the source property key (not the local name) for renames,
    // and `has_default` should propagate when this property carried the
    // default at its own level even if the local was a sub-pattern.
    if let Some(key) = prop_key {
        // `{ foo: alias }` or `{ foo: alias = default }` — the destructure
        // pulls `foo` from $props() and binds it locally as `alias`. Only
        // the immediate first entry corresponds to this property; deeper
        // entries belong to sub-patterns and keep their own key.
        if let Some(first) = out.get_mut(before) {
            if !prop.shorthand {
                first.prop_key = key;
            }
        }
    }
    if has_default {
        for entry in &mut out[before..] {
            entry.has_default = true;
            if bindable {
                entry.is_bindable = true;
            }
            if entry.default_type_text.is_none() {
                entry.default_type_text = inferred_default.clone();
            }
        }
    }
}

fn collect_rest(pat: &BindingPattern<'_>, out: &mut Vec<PropInfo>, is_rest: bool) {
    match pat {
        BindingPattern::BindingIdentifier(id) => {
            let name = SmolStr::from(id.name.as_str());
            out.push(PropInfo {
                local_name: name.clone(),
                prop_key: name,
                range: Range::new(id.span.start, id.span.end),
                is_rest,
                has_default: false,
                is_bindable: false,
                default_type_text: None,
            });
        }
        // Rest patterns holding further destructuring are allowed but
        // unusual; walk recursively.
        other => collect_from_binding(other, out),
    }
}

/// True when `expr` is a `$bindable(…)` call (with or without args).
/// Walks through parenthesised wrappers so `($bindable())` still
/// matches.
fn is_bindable_call(expr: &Expression<'_>) -> bool {
    match expr {
        Expression::CallExpression(call) => matches!(
            &call.callee,
            Expression::Identifier(id) if id.name == "$bindable",
        ),
        Expression::ParenthesizedExpression(p) => is_bindable_call(&p.expression),
        _ => false,
    }
}

/// Infer a TypeScript type from a JS literal default-value expression.
/// Mirrors upstream svelte2tsx's `getTypeForDefault` for the common
/// cases. Returns `None` for unrecognised expressions; callers fall
/// back to `any` in the synthesised typedef.
///
/// Notably `null` and `undefined` default values widen to `None`
/// (caller emits `any`) rather than the literal `null` / `undefined`
/// type — consumers passing real values would otherwise fail. Matches
/// upstream's behaviour for the same reason: `let { x = null } =
/// $props()` is the canonical "no real default" pattern, and binding
/// the prop type to `null` is almost always wrong.
fn infer_default_type(expr: &Expression<'_>) -> Option<SmolStr> {
    match expr {
        Expression::StringLiteral(_) | Expression::TemplateLiteral(_) => {
            Some(SmolStr::new_static("string"))
        }
        Expression::NumericLiteral(_) | Expression::BigIntLiteral(_) => {
            Some(SmolStr::new_static("number"))
        }
        Expression::BooleanLiteral(_) => Some(SmolStr::new_static("boolean")),
        // `null` and `undefined` defaults: widen to `any` (upstream
        // `getTypeForDefault` does the same — these are placeholder
        // defaults, not assertions about the real type).
        Expression::NullLiteral(_) => None,
        Expression::Identifier(id) if id.name == "undefined" => None,
        Expression::ArrayExpression(_) => Some(SmolStr::new_static("any[]")),
        Expression::ObjectExpression(_) => Some(SmolStr::new_static("Record<string, any>")),
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => {
            Some(SmolStr::new_static("Function"))
        }
        Expression::CallExpression(call) => {
            // `$bindable()` and `$bindable(default)` — recurse on the
            // first arg when present, otherwise unknown.
            let is_bindable = matches!(
                &call.callee,
                Expression::Identifier(id) if id.name == "$bindable",
            );
            if is_bindable {
                call.arguments
                    .first()
                    .and_then(|a| a.as_expression().and_then(infer_default_type))
            } else {
                None
            }
        }
        Expression::ParenthesizedExpression(p) => infer_default_type(&p.expression),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::{ScriptLang, parse_script_body};

    fn props(src: &str) -> Vec<String> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        PropsInfo::build(&parsed.program, src)
            .destructures
            .into_iter()
            .map(|p| p.local_name.to_string())
            .collect()
    }

    #[test]
    fn empty_script_returns_empty() {
        assert!(props("").is_empty());
    }

    #[test]
    fn no_props_call_returns_empty() {
        assert!(props("const x = 1;").is_empty());
    }

    #[test]
    fn simple_shorthand_prop() {
        assert_eq!(props("let { foo } = $props();"), vec!["foo"]);
    }

    #[test]
    fn multiple_shorthand_props() {
        assert_eq!(props("let { a, b, c } = $props();"), vec!["a", "b", "c"]);
    }

    #[test]
    fn prop_with_default() {
        assert_eq!(props("let { foo = 'bar' } = $props();"), vec!["foo"]);
    }

    #[test]
    fn renamed_prop_returns_local_name() {
        // `{ class: classValue }` — local binding is `classValue`, NOT `class`.
        // Using `class` would produce `void class;` which is a JS reserved-
        // word error. The local_name is what we record.
        assert_eq!(
            props("let { class: classValue } = $props();"),
            vec!["classValue"]
        );
    }

    #[test]
    fn renamed_with_default() {
        assert_eq!(
            props("let { class: classValue = 'default' } = $props();"),
            vec!["classValue"]
        );
    }

    #[test]
    fn rest_prop() {
        let src = "let { foo, ...rest } = $props();";
        assert_eq!(props(src), vec!["foo", "rest"]);
    }

    #[test]
    fn rest_is_flagged_on_info() {
        let src = "let { a, ...rest } = $props();";
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        let info = PropsInfo::build(&parsed.program, src).destructures;
        assert_eq!(info.len(), 2);
        assert!(!info[0].is_rest);
        assert!(info[1].is_rest);
    }

    #[test]
    fn typed_destructuring() {
        assert_eq!(
            props("let { foo, bar }: { foo: string; bar: number } = $props();"),
            vec!["foo", "bar"]
        );
    }

    #[test]
    fn generic_props_call() {
        assert_eq!(
            props("let { foo } = $props<{ foo: string }>();"),
            vec!["foo"]
        );
    }

    #[test]
    fn props_dot_id_not_recognized_as_props_call() {
        // $props.id() is a different rune; `foo` there isn't a component prop.
        assert!(props("let foo = $props.id();").is_empty());
    }

    #[test]
    fn props_not_at_top_level_ignored() {
        // $props() inside a function isn't valid Svelte; don't extract.
        let src = "function f() { let { foo } = $props(); }";
        assert!(props(src).is_empty());
    }

    #[test]
    fn comment_between_destructured_props() {
        let src = "let {\n  a,\n  /* b comment */\n  b,\n  // c comment\n  c,\n} = $props();";
        assert_eq!(props(src), vec!["a", "b", "c"]);
    }

    #[test]
    fn generics_in_bindable_default() {
        // $bindable<Record<string, number>>({}) — generic args with commas
        // inside < > which trips character-level parsers but not oxc.
        let src = "let { members = $bindable<Record<string, number>>({}), count = 0 } = $props();";
        assert_eq!(props(src), vec!["members", "count"]);
    }

    #[test]
    fn prop_name_with_dollar_suffix() {
        assert_eq!(props("let { parent$ } = $props();"), vec!["parent$"]);
    }

    #[test]
    fn nested_destructuring_recurses() {
        // let { outer: { inner } } = $props() — inner is a leaf binding.
        let src = "let { outer: { inner } } = $props();";
        assert_eq!(props(src), vec!["inner"]);
    }

    #[test]
    fn ranges_point_at_local_identifier() {
        let src = "let { foo } = $props();";
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        let info = PropsInfo::build(&parsed.program, src).destructures;
        assert_eq!(info.len(), 1);
        assert_eq!(info[0].range.slice(src), "foo");
    }

    fn props_type(src: &str) -> Option<String> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        PropsInfo::build(&parsed.program, src).type_text
    }

    #[test]
    fn props_type_from_destructure_annotation() {
        let src = "let { foo }: { foo: string } = $props();";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn props_type_from_generic_arg() {
        let src = "let { foo } = $props<{ foo: string }>();";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn props_type_none_for_untyped_props_call() {
        assert_eq!(props_type("let { foo } = $props();"), None);
    }

    #[test]
    fn synth_props_type_from_single_export_let_with_type() {
        let src = "export let b: (v: string) => void;";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ b: (v: string) => void; }")
        );
    }

    #[test]
    fn synth_props_type_makes_default_initialized_optional() {
        let src = "export let count: number = 0;";
        assert_eq!(props_type(src).as_deref(), Some("{ count?: number; }"));
    }

    #[test]
    fn synth_props_type_no_type_no_default_is_any_required() {
        let src = "export let data;";
        assert_eq!(props_type(src).as_deref(), Some("{ data: any; }"));
    }

    #[test]
    fn synth_props_type_no_type_with_default_is_typeof_optional() {
        // Unannotated-with-initializer emits `typeof <name>` so
        // downstream consumers see the initializer-inferred type
        // (number in this case) rather than collapsing to `any`.
        // `has_init = true` path in `append_props_from_var_decl`.
        let src = "export let count = 42;";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ count?: typeof count; }")
        );
    }

    #[test]
    fn synth_props_type_multiple_export_lets() {
        let src = "export let a: string;\nexport let b: number = 1;";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ a: string; b?: number; }")
        );
    }

    #[test]
    fn synth_props_type_treats_export_const_like_export_let() {
        // `export const foo: T = v` → read-only prop; still contributes
        // a (optional) property to the synthesized type.
        let src = "export const foo: string = 'x';";
        assert_eq!(props_type(src).as_deref(), Some("{ foo?: string; }"));
    }

    #[test]
    fn props_call_wins_over_export_let() {
        // A script with BOTH `$props()` (annotated) and a stray `export
        // let` should prefer the explicit `$props()` annotation.
        let src = "export let stray: number;\nlet { foo }: { foo: string } = $props();";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn export_fn_does_not_contribute_props() {
        // `export function` is an exported helper, not a prop; ignored.
        let src = "export function helper() {}";
        assert_eq!(props_type(src), None);
    }

    #[test]
    fn export_type_alias_does_not_contribute_props() {
        let src = "export type Foo = number;";
        assert_eq!(props_type(src), None);
    }

    #[test]
    fn dollar_dollar_props_fallback_when_no_props_call() {
        // Svelte-4 `interface $$Props { ... }` convention. With no
        // `$props()` call, $$Props is the Props type source.
        let src = "interface $$Props { foo: number }";
        assert_eq!(props_type(src).as_deref(), Some("$$Props"));
    }

    #[test]
    fn props_call_wins_over_dollar_dollar_props() {
        // If both are present, the explicit `$props()` annotation wins.
        let src = "interface $$Props { foo: number }\nlet { bar }: { bar: string } = $props();";
        assert_eq!(props_type(src).as_deref(), Some("{ bar: string }"));
    }

    #[test]
    fn export_let_wins_over_nothing_but_not_over_dollar_dollar_props() {
        // $$Props wins over the export-let fallback (both are Svelte-4
        // conventions, but $$Props is more explicit and newer).
        let src = "interface $$Props { foo: number }\nexport let stray: number;";
        assert_eq!(props_type(src).as_deref(), Some("$$Props"));
    }

    #[test]
    fn export_specifier_rename_contributes_class_prop() {
        // Svelte 4 pattern: rename a local to a JS reserved name so it
        // can be used as a component prop.
        let src = "let className: any = \"\";\nexport { className as class };";
        assert_eq!(props_type(src).as_deref(), Some("{ class?: any; }"));
    }

    #[test]
    fn export_specifier_preserves_local_type() {
        let src = "let n: number = 0;\nexport { n as count };";
        assert_eq!(props_type(src).as_deref(), Some("{ count?: number; }"));
    }

    #[test]
    fn export_specifier_missing_init_marks_required() {
        let src = "let n: number;\nexport { n as count };";
        assert_eq!(props_type(src).as_deref(), Some("{ count: number; }"));
    }

    #[test]
    fn export_specifier_missing_local_falls_back_to_any() {
        // Export of an undeclared local — pathological but don't panic.
        let src = "export { missing as foo };";
        assert_eq!(props_type(src).as_deref(), Some("{ foo: any; }"));
    }

    #[test]
    fn export_specifier_combined_with_export_let() {
        let src =
            "export let width = 40;\nlet className: any = \"\";\nexport { className as class };";
        assert_eq!(
            props_type(src).as_deref(),
            Some("{ width?: typeof width; class?: any; }")
        );
    }

    // ---------- PropsInfo::build tests ----------

    fn build(src: &str) -> PropsInfo {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        PropsInfo::build(&parsed.program, src)
    }

    #[test]
    fn props_info_default_is_none() {
        let info = build("");
        assert_eq!(info.source, PropsSource::None);
        assert_eq!(info.type_text, None);
        assert_eq!(info.type_root_name, None);
        assert!(info.destructures.is_empty());
    }

    #[test]
    fn props_info_rune_annotation_source() {
        let info = build("let { foo }: { foo: string } = $props();");
        assert_eq!(info.source, PropsSource::RuneAnnotation);
        assert_eq!(info.type_text.as_deref(), Some("{ foo: string }"));
        assert_eq!(info.type_root_name, None); // literal shape
        let names: Vec<&str> = info
            .destructures
            .iter()
            .map(|p| p.local_name.as_str())
            .collect();
        assert_eq!(names, vec!["foo"]);
    }

    #[test]
    fn props_info_rune_generic_source() {
        let info = build("let { a, b } = $props<Props>();");
        assert_eq!(info.source, PropsSource::RuneGeneric);
        assert_eq!(info.type_text.as_deref(), Some("Props"));
        assert_eq!(info.type_root_name.as_deref(), Some("Props"));
        let names: Vec<&str> = info
            .destructures
            .iter()
            .map(|p| p.local_name.as_str())
            .collect();
        assert_eq!(names, vec!["a", "b"]);
    }

    #[test]
    fn props_info_rune_generic_with_type_parameters() {
        // Generic-instantiated Props type → type_root_name should
        // still be the leading identifier, not include the `<T>` part.
        let info = build("let { items } = $props<ListProps<string>>();");
        assert_eq!(info.source, PropsSource::RuneGeneric);
        assert_eq!(info.type_text.as_deref(), Some("ListProps<string>"));
        assert_eq!(info.type_root_name.as_deref(), Some("ListProps"));
    }

    #[test]
    fn props_info_legacy_interface() {
        let info = build("interface $$Props { foo: number }");
        assert_eq!(info.source, PropsSource::LegacyInterface);
        assert_eq!(info.type_text.as_deref(), Some("$$Props"));
        assert_eq!(info.type_root_name.as_deref(), Some("$$Props"));
        assert!(info.destructures.is_empty());
    }

    #[test]
    fn props_info_synthesised_from_export_let() {
        let info = build("export let width: number;\nexport let count = 0;");
        assert_eq!(info.source, PropsSource::SynthesisedFromExports);
        assert_eq!(
            info.type_text.as_deref(),
            Some("{ width: number; count?: typeof count; }")
        );
        assert_eq!(info.type_root_name, None); // literal shape
        assert!(info.destructures.is_empty());
    }

    #[test]
    fn props_info_rune_annotation_wins_over_export_let() {
        // Priority order: explicit $props() annotation beats a stray
        // export let (pathological but possible in migration code).
        let info = build("export let stray: number;\nlet { foo }: { foo: string } = $props();");
        assert_eq!(info.source, PropsSource::RuneAnnotation);
        assert_eq!(info.type_text.as_deref(), Some("{ foo: string }"));
    }

    #[test]
    fn props_info_rune_annotation_wins_over_legacy_interface() {
        let info =
            build("interface $$Props { foo: number }\nlet { bar }: { bar: string } = $props();");
        assert_eq!(info.source, PropsSource::RuneAnnotation);
        assert_eq!(info.type_text.as_deref(), Some("{ bar: string }"));
    }

    #[test]
    fn props_info_legacy_interface_wins_over_export_let() {
        let info = build("interface $$Props { foo: number }\nexport let stray: number;");
        assert_eq!(info.source, PropsSource::LegacyInterface);
        assert_eq!(info.type_text.as_deref(), Some("$$Props"));
    }

    #[test]
    fn props_info_untyped_props_call_has_destructures_but_no_type() {
        let info = build("let { foo, bar } = $props();");
        assert_eq!(info.source, PropsSource::None);
        assert_eq!(info.type_text, None);
        let names: Vec<&str> = info
            .destructures
            .iter()
            .map(|p| p.local_name.as_str())
            .collect();
        assert_eq!(names, vec!["foo", "bar"]);
    }

    #[test]
    fn root_type_name_of_handles_common_shapes() {
        assert_eq!(root_type_name_of("Props").as_deref(), Some("Props"));
        assert_eq!(root_type_name_of("Props<T>").as_deref(), Some("Props"));
        assert_eq!(
            root_type_name_of("ChannelMessageProps").as_deref(),
            Some("ChannelMessageProps")
        );
        assert_eq!(root_type_name_of("  Props").as_deref(), Some("Props"));
        assert_eq!(root_type_name_of("$$Props").as_deref(), Some("$$Props"));
        assert_eq!(root_type_name_of("_Private").as_deref(), Some("_Private"));
        // Literal shapes / tuples / unions start with non-identifier chars.
        assert_eq!(root_type_name_of("{ foo: string }"), None);
        assert_eq!(root_type_name_of("[string, number]"), None);
        assert_eq!(root_type_name_of(""), None);
    }
}
