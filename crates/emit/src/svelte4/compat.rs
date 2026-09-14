//! Svelte-4 compatibility helpers.
//!
//! Two concerns live here:
//!   1. Detection — `is_svelte4_component`, the various
//!      `has_*` / `contains_*` / `is_runes_mode` predicates that decide
//!      whether to apply the rewrites or widening intersections.
//!   2. Source-text rewrites of the script body (definite-assignment
//!      `!`, de-narrowing reassignments, untyped-export widening) and
//!      the `$$slots` / `$$props` / `$$restProps` ambient emission.
//!
//! When Svelte 4 is officially retired this whole module gets deleted
//! along with the `// SVELTE-4-COMPAT` callsites in `lib.rs`. See
//! `design/phase_g/DESIGN.md`.

use std::ops::Range;

use smol_str::SmolStr;
use svn_parser::ScriptLang;

use crate::sveltekit;

/// A top-level `let` declarator in the instance script body. Byte
/// positions are absolute offsets into the emit buffer.
struct LetDeclarator {
    name: SmolStr,
    /// Position right after the binding name, where `!` or `: T` goes.
    name_end: usize,
    has_type_annotation: bool,
    /// Already carries a `!` definite-assignment assertion.
    definite: bool,
    init: DeclaratorInit,
    /// Position right after the whole `let …` statement.
    stmt_end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclaratorInit {
    Absent,
    /// `= undefined` or `= null`.
    Nullish,
    Other,
}

/// Parse the script body spliced at `body` inside `out` and list its
/// top-level `let` declarators that bind a plain identifier.
///
/// The in-place rewrites need to know where a declaration's name, type
/// annotation and statement end sit. A byte scan misreads text that
/// only looks like code — the word `let` in a comment followed by an
/// unclosed `(`, an initializer continued onto the next line by a
/// trailing `+` — so the positions come from the parser. Only
/// top-level statements count: every rewrite target is a
/// component-scope binding, and a nested `let` with the same name is a
/// different variable.
fn collect_top_level_lets(out: &str, body: &Range<usize>) -> Vec<LetDeclarator> {
    use oxc_ast::ast::{
        BindingPattern, Declaration, Expression, Statement, VariableDeclarationKind,
    };

    let Some(src) = out.get(body.clone()) else {
        return Vec::new();
    };
    let alloc = oxc_allocator::Allocator::default();
    // Always parse as TypeScript. It accepts every JS body, and a JS
    // overlay's body can still hold TS-only syntax (a `lang`-less script
    // with a type annotation) that would make a JS parse give up.
    let parsed = svn_parser::parse_script_body(&alloc, src, ScriptLang::Ts);
    if parsed.panicked {
        return Vec::new();
    }
    let mut decls = Vec::new();
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(d) => d,
            Statement::ExportDeclaration(e) => match &e.declaration {
                Declaration::VariableDeclaration(d) => d,
                _ => continue,
            },
            // None of these can declare a `let`.
            Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportDefaultDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSEnumDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_)
            | Statement::TSNamespaceDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSImportEqualsDeclaration(_)
            | Statement::TSExportAssignment(_)
            | Statement::TSNamespaceExportDeclaration(_) => continue,
            svn_analyze::non_declaration_statement!() => continue,
        };
        if decl.kind != VariableDeclarationKind::Let {
            continue;
        }
        let stmt_end = body.start + decl.span.end as usize;
        for d in &decl.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            let init = match &d.init {
                None => DeclaratorInit::Absent,
                Some(Expression::NullLiteral(_)) => DeclaratorInit::Nullish,
                Some(Expression::Identifier(i)) if i.name.as_str() == "undefined" => {
                    DeclaratorInit::Nullish
                }
                Some(_) => DeclaratorInit::Other,
            };
            decls.push(LetDeclarator {
                name: SmolStr::from(id.name.as_str()),
                name_end: body.start + id.span.end as usize,
                has_type_annotation: d.type_annotation.is_some(),
                definite: d.definite,
                init,
                stmt_end,
            });
        }
    }
    decls
}

fn is_target(target_names: &[SmolStr], name: &str) -> bool {
    target_names.iter().any(|t| t == name)
}

/// Splice `(position, text)` insertions into `out` in one rebuild.
/// Positions must be ascending. Returns the edits as `(position,
/// length)` pairs in pre-rewrite coordinates, the shape
/// `EmitBuffer::adjust_token_map_for_insertions` re-anchors with.
fn splice_insertions(out: &mut String, insertions: &[(usize, String)]) -> Vec<(u32, u32)> {
    if insertions.is_empty() {
        return Vec::new();
    }
    let original = std::mem::take(out);
    let extra: usize = insertions.iter().map(|(_, text)| text.len()).sum();
    let mut rebuilt = String::with_capacity(original.len() + extra);
    let mut edits = Vec::with_capacity(insertions.len());
    let mut cursor = 0;
    for (pos, text) in insertions {
        rebuilt.push_str(&original[cursor..*pos]);
        rebuilt.push_str(text);
        edits.push((*pos as u32, text.len() as u32));
        cursor = *pos;
    }
    rebuilt.push_str(&original[cursor..]);
    *out = rebuilt;
    edits
}

/// Rewrite `let <name>: T;` → `let <name>!: T;` for each target
/// declared at the top level of the script body at `body`.
///
/// Svelte assigns these at runtime (a parent passes the prop, a
/// `bind:this` element mounts), but TypeScript's flow analysis can't
/// see that, so any read would be flagged "used before being
/// assigned" (TS2454). The `!:` definite-assignment assertion tells
/// TypeScript to trust us.
///
/// Only typed declarators without an initializer qualify: an untyped
/// one has no annotation to attach `!` to, and `!` next to an
/// initializer is itself an error (TS1263).
pub(crate) fn rewrite_definite_assignment_in_place(
    out: &mut String,
    body: &Range<usize>,
    target_names: &[SmolStr],
) -> Vec<(u32, u32)> {
    if target_names.is_empty() {
        return Vec::new();
    }
    let insertions: Vec<(usize, String)> = collect_top_level_lets(out, body)
        .into_iter()
        .filter(|d| {
            d.has_type_annotation
                && !d.definite
                && d.init == DeclaratorInit::Absent
                && is_target(target_names, &d.name)
        })
        .map(|d| (d.name_end, String::from("!")))
        .collect();
    splice_insertions(out, &insertions)
}

/// Does the parsed template fragment contain a `<slot>` element?
///
/// Replaces an earlier `doc.source.contains("<slot")` substring check.
/// The AST walk is strictly more accurate:
/// - Correctly matches only `<slot>` / `<slot name="x">` (tag name is
///   exactly `slot`), not `<slotfoo>` or `<Slot>`.
/// - Skips comments and string content — those produce Text / Comment
///   nodes, not Element nodes.
/// - Recurses into all block children (if/each/await/key/snippet)
///   and nested elements so a `<slot>` inside a branch of an
///   `{#if}` is detected.
pub(crate) fn fragment_contains_slot(fragment: &svn_parser::Fragment) -> bool {
    use svn_parser::Node;
    for node in &fragment.nodes {
        match node {
            Node::Element(e) => {
                if e.name.as_str() == "slot" {
                    return true;
                }
                if fragment_contains_slot(&e.children) {
                    return true;
                }
            }
            Node::Component(c) => {
                if fragment_contains_slot(&c.children) {
                    return true;
                }
            }
            Node::SvelteElement(e) => {
                if fragment_contains_slot(&e.children) {
                    return true;
                }
            }
            Node::IfBlock(b) => {
                if fragment_contains_slot(&b.consequent) {
                    return true;
                }
                for arm in &b.elseif_arms {
                    if fragment_contains_slot(&arm.body) {
                        return true;
                    }
                }
                if let Some(alt) = &b.alternate
                    && fragment_contains_slot(alt)
                {
                    return true;
                }
            }
            Node::EachBlock(b) => {
                if fragment_contains_slot(&b.body) {
                    return true;
                }
                if let Some(alt) = &b.alternate
                    && fragment_contains_slot(alt)
                {
                    return true;
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(p) = &b.pending
                    && fragment_contains_slot(p)
                {
                    return true;
                }
                if let Some(t) = &b.then_branch
                    && fragment_contains_slot(&t.body)
                {
                    return true;
                }
                if let Some(c) = &b.catch_branch
                    && fragment_contains_slot(&c.body)
                {
                    return true;
                }
            }
            Node::KeyBlock(b) => {
                if fragment_contains_slot(&b.body) {
                    return true;
                }
            }
            Node::SnippetBlock(b) => {
                if fragment_contains_slot(&b.body) {
                    return true;
                }
            }
            Node::Text(_) | Node::Comment(_) | Node::Interpolation(_) => {}
        }
    }
    false
}

/// SVELTE-4-COMPAT — v0.3 Item 3. Detect whether the component has a
/// `$$Events` interface or type declaration in its instance or module
/// script via an AST walk over the parsed scripts. When true, the
/// default-export declaration intersects with
/// `& { readonly __svn_events: $$Events }` so consumers resolve to
/// `__svn_ensure_component`'s typed overload and get narrowed
/// `$on("evt", handler)` signatures.
///
/// Reviewer follow-up #3a: pre-fix this scanned the raw source for
/// the substrings `"interface $$Events"` / `"type $$Events "`
/// (with trailing space), which (a) false-fired on comments and
/// string literals containing those phrases, (b) silently missed
/// `type $$Events=…` (no whitespace before `=`) and `type
/// $$Events<T>=…` (generic), and (c) missed
/// `export interface $$Events { … }` in some forms. AST-based
/// detection covers every well-formed declaration shape and stays
/// blind to comments / strings / unrelated identifiers.
pub(crate) fn has_strict_events_ast(
    parsed_instance: Option<&svn_parser::ParsedScript<'_>>,
    _parsed_module: Option<&svn_parser::ParsedScript<'_>>,
) -> bool {
    // Reviewer follow-up #5 (round 4): pre-fix this also scanned
    // the module script. Upstream svelte2tsx
    // (`processModuleScriptTag.ts:119`) explicitly REJECTS a
    // `$$Events` declaration in `<script module>` — only the
    // instance script's declaration is the strict-event opt-in.
    // Recognizing the module-script form would silently widen
    // the event surface based on a declaration upstream wouldn't
    // allow.
    //
    // We don't currently surface a diagnostic for the
    // module-script case (lint-level concern, deferred). Ignore
    // the module-script declaration so the rest of the pipeline
    // behaves as if `$$Events` is absent.
    let scan = |program: &oxc_ast::ast::Program<'_>| -> bool {
        program.body.iter().any(statement_declares_events)
    };
    parsed_instance.is_some_and(|p| scan(&p.program))
}

/// True when `stmt` is an `interface $$Events` or `type $$Events`
/// declaration (including the `export interface` / `export type`
/// re-exporting forms). Used by [`has_strict_events_ast`].
fn statement_declares_events(stmt: &oxc_ast::ast::Statement<'_>) -> bool {
    statement_declares_named_type(stmt, "$$Events")
}

/// SlotHandler PLAN Stage 5: detect an `interface $$Slots` /
/// `type $$Slots` declaration in the instance script via AST. When
/// present, the render-fn return's `slots:` field uses `$$Slots`
/// instead of the synthesised slot-defs (mirrors upstream's
/// `uses$$SlotsInterface` flag in `createRenderFunction.ts:125`).
///
/// Module-script declarations don't count — upstream's
/// `processModuleScriptTag.ts:127` rejects `$$Slots` there. Same
/// rule we apply for `$$Events`.
pub(crate) fn has_strict_slots_ast(parsed_instance: Option<&svn_parser::ParsedScript<'_>>) -> bool {
    parsed_instance.is_some_and(|p| {
        p.program
            .body
            .iter()
            .any(|s| statement_declares_named_type(s, "$$Slots"))
    })
}

/// Generic helper: true when `stmt` is an `interface NAME` or
/// `type NAME` declaration (with or without `export`).
///
/// Top-level-only contract: only the two type-declaration shapes (and
/// their `export`-wrapped forms) can satisfy the check — every other
/// declaration kind is enumerated below so the match stays exhaustive
/// and a new oxc `Statement` variant fails compilation instead of
/// silently reading as "not a type declaration".
fn statement_declares_named_type(stmt: &oxc_ast::ast::Statement<'_>, name: &str) -> bool {
    use oxc_ast::ast::{Declaration, Statement};
    match stmt {
        Statement::TSInterfaceDeclaration(d) => d.id.name.as_str() == name,
        Statement::TSTypeAliasDeclaration(d) => d.id.name.as_str() == name,
        Statement::ExportDeclaration(e) => match &e.declaration {
            Declaration::TSInterfaceDeclaration(d) => d.id.name.as_str() == name,
            Declaration::TSTypeAliasDeclaration(d) => d.id.name.as_str() == name,
            _ => false,
        },
        Statement::ExportNamedDeclaration(_) | Statement::ExportFromDeclaration(_) => false,
        Statement::VariableDeclaration(_)
        | Statement::FunctionDeclaration(_)
        | Statement::ClassDeclaration(_)
        | Statement::ImportDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::TSEnumDeclaration(_)
        | Statement::TSExternalModuleDeclaration(_)
        | Statement::TSNamespaceDeclaration(_)
        | Statement::TSGlobalDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_)
        | Statement::TSExportAssignment(_)
        | Statement::TSNamespaceExportDeclaration(_) => false,
        svn_analyze::non_declaration_statement!() => false,
    }
}

/// SVELTE-4-COMPAT: Detect the `<script strictEvents>` bare attribute
/// that upstream svelte2tsx uses as a user opt-in for event-typing
/// narrowing without requiring a `$$Events` interface. One of the
/// three triggers that turns on event narrowing.
pub(crate) fn has_strict_events_attr(doc: &svn_parser::Document<'_>) -> bool {
    doc.instance_script.as_ref().is_some_and(|s| {
        s.attrs
            .iter()
            .any(|a| a.name.eq_ignore_ascii_case("strictEvents") && a.value.is_none())
    })
}

/// Infer Svelte 5 runes mode the way upstream svelte2tsx does
/// (`ExportedNames.isRunesMode`): the script or a template expression
/// references one of the `$state` / `$derived` / `$effect` globals,
/// something is declared from `$props()`, an `await` sits outside any
/// function, or the document sets `<svelte:options runes>`. Nothing
/// else counts — a rune name in markup text or a comment is not a
/// reference, and `$inspect` alone does not switch modes.
///
/// A `$state` that resolves to a store subscription (`const state =
/// writable(…)` in a Svelte-4 component) is not a global either, so
/// the top-level bindings of both scripts are excluded first.
pub(crate) fn is_runes_mode(
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
    parsed_instance: Option<&svn_parser::ParsedScript<'_>>,
    parsed_module: Option<&svn_parser::ParsedScript<'_>>,
) -> bool {
    let source = doc.source;
    // An explicit `<svelte:options runes>` / `runes={true}` forces runes
    // ON. `runes={false}` cannot force runes OFF here (upstream OR-s the
    // option with the script signals), so only `Some(true)` participates.
    if svn_parser::runes_option(fragment, source) == Some(true) {
        return true;
    }
    let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();
    for parsed in [parsed_instance, parsed_module].into_iter().flatten() {
        svn_analyze::collect_top_level_bindings(&parsed.program, &mut bound);
    }
    let mut probe = svn_analyze::RunesProbe::new(svn_analyze::RunesRule::Svelte2tsx, &bound);
    for parsed in [parsed_instance, parsed_module].into_iter().flatten() {
        probe.scan_program(&parsed.program);
        if probe.found {
            return true;
        }
    }
    // Template expressions: `{let x = $state(0)}`, `{await p}`. Only
    // worth parsing when the text carries a marker at all.
    let has_marker = doc.template.text_runs.iter().any(|run| {
        source
            .get(run.start as usize..run.end as usize)
            .is_some_and(|t| {
                ["$state", "$derived", "$effect", "await"]
                    .iter()
                    .any(|m| t.contains(m))
            })
    });
    if !has_marker {
        return false;
    }
    let alloc = oxc_allocator::Allocator::default();
    for expr in svn_analyze::template_expression_ranges(fragment) {
        let Some(text) = source.get(expr.range.start as usize..expr.range.end as usize) else {
            continue;
        };
        let wrapped = if expr.is_declaration {
            format!("let {text}\n;")
        } else {
            format!("({text}\n);")
        };
        let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
        probe.scan_program(&parsed.program);
        if probe.found {
            return true;
        }
    }
    false
}

/// Append ` NAME = undefined as any;` after each top-level `let`
/// statement that declares an initialized target.
///
/// `export let size: Size = 'medium'` narrows `size` to the literal, so
/// a later `size === 'large'` fires TS2367 ("no overlap"). Assigning
/// `any` right after the declaration widens it back to the annotation.
pub(crate) fn denarrow_typed_exported_props_in_place(
    out: &mut String,
    body: &Range<usize>,
    target_names: &[SmolStr],
) -> Vec<(u32, u32)> {
    if target_names.is_empty() {
        return Vec::new();
    }
    let mut insertions: Vec<(usize, String)> = Vec::new();
    for d in collect_top_level_lets(out, body) {
        if d.init == DeclaratorInit::Absent || !is_target(target_names, &d.name) {
            continue;
        }
        // Declarators of one statement share its trailer.
        if insertions.last().map(|(pos, _)| *pos) != Some(d.stmt_end) {
            let lead = if out[..d.stmt_end].ends_with(';') {
                ""
            } else {
                ";"
            };
            insertions.push((d.stmt_end, String::from(lead)));
        }
        if let Some((_, trailer)) = insertions.last_mut() {
            trailer.push(' ');
            trailer.push_str(&d.name);
            trailer.push_str(" = undefined as any;");
        }
    }
    splice_insertions(out, &insertions)
}

/// SVELTE-4-COMPAT: emit `let $$slots = …; let $$props = …; let
/// $$restProps = …;` at the top of the render function for each
/// ambient the component refers to (an identifier reference in a
/// script or template expression — see `svn_analyze::find_ambient_refs`).
///
/// Types: `Record<string, any>` for all three. Upstream's
/// `__sveltets_2_slotsType({…slot names…})` is more precise (each
/// slot is typed as `boolean | ''`), but that requires walking the
/// template to collect slot names and emit a shape literal.
pub(crate) fn emit_svelte4_ambients(out: &mut String, refs: svn_analyze::AmbientRefs, is_ts: bool) {
    // In TS overlays we emit inline `: T` annotations. In JS overlays
    // we must not — tsgo fires TS8010 and aborts project-wide once
    // hit, silently suppressing every legitimate diagnostic
    // elsewhere. Emit JSDoc casts on the RHS for JS overlays.
    if refs.slots {
        if is_ts {
            out.push_str("    let $$slots: Record<string, boolean | undefined> = {};\n");
        } else {
            out.push_str(
                "    let $$slots = /** @type {Record<string, boolean | undefined>} */ ({});\n",
            );
        }
        out.push_str("    void $$slots;\n");
    }
    if refs.rest_props {
        if is_ts {
            out.push_str("    let $$restProps: Record<string, any> = {};\n");
        } else {
            out.push_str("    let $$restProps = /** @type {Record<string, any>} */ ({});\n");
        }
        out.push_str("    void $$restProps;\n");
    }
    if refs.props {
        if is_ts {
            out.push_str("    let $$props: Record<string, any> = {};\n");
        } else {
            out.push_str("    let $$props = /** @type {Record<string, any>} */ ({});\n");
        }
        out.push_str("    void $$props;\n");
    }
}
/// JS-overlay equivalent of `rewrite_definite_assignment_in_place` +
/// `widen_untyped_exported_props_in_place` rolled into one. For each
/// `let NAME[, NAME…];` declaration where NAME is a target AND that
/// declarator has no initializer, splice `= /** @type {any} */ (null)`
/// between NAME (or its type annotation) and the terminator — turning
/// `let b;` into `let b = /** @type {any} */ (null);`.
///
/// Fixes three TS-strict-mode JS-overlay diagnostics in one pass:
///   - TS7034/TS7005 on the declaration ("variable implicitly any in
///     some locations") — the initializer's `any` gives TS an explicit
///     type for subsequent flow.
///   - TS2454 on later reads ("used before being assigned") — the
///     initializer satisfies definite-assign flow.
///   - TS2367/TS2322 on type-check expressions that would have
///     otherwise narrowed against a body-local `undefined`-inferred
///     type.
///
/// User-authored JSDoc `/** @type {T} */` preceding the declaration is
/// preserved and takes priority: TS reads user's `@type` to declare
/// NAME as `T`, the initializer's `any` is assignable to `T` via JS-loose
/// rules, no TS2322 secondary fires.
pub(crate) fn widen_untyped_exports_jsdoc_in_place(
    out: &mut String,
    body: &Range<usize>,
    target_names: &[SmolStr],
    route_kind: Option<sveltekit::RouteKind>,
) -> Vec<(u32, u32)> {
    let insertions: Vec<(usize, String)> = collect_widening_sites(out, body, target_names)
        .into_iter()
        .map(|(pos, name)| {
            let text = match route_kind.and_then(|k| sveltekit::kit_widen_type(&name, k)) {
                Some(ty) => format!(" = /** @type {{{ty}}} */ (/** @type {{any}} */ (null))"),
                None => String::from(" = /** @type {any} */ (null)"),
            };
            (pos, text)
        })
        .collect();
    splice_insertions(out, &insertions)
}

/// Rewrite `let <name>;` → `let <name>: any;` (or the SvelteKit route
/// type for `data` / `form`) for each untyped target, so an untyped
/// Svelte-4 prop doesn't fire TS7034/TS7005.
pub(crate) fn widen_untyped_exported_props_in_place(
    out: &mut String,
    body: &Range<usize>,
    target_names: &[SmolStr],
    route_kind: Option<sveltekit::RouteKind>,
) -> Vec<(u32, u32)> {
    let insertions: Vec<(usize, String)> = collect_widening_sites(out, body, target_names)
        .into_iter()
        .map(|(pos, name)| {
            let ty = route_kind
                .and_then(|k| sveltekit::kit_widen_type(&name, k))
                .unwrap_or("any");
            (pos, format!(": {ty}"))
        })
        .collect();
    splice_insertions(out, &insertions)
}

/// Shared site list for the two widening rewrites: the name end of
/// every untyped target declared without an initializer (or with a
/// bare `undefined` / `null` one).
fn collect_widening_sites(
    out: &str,
    body: &Range<usize>,
    target_names: &[SmolStr],
) -> Vec<(usize, SmolStr)> {
    if target_names.is_empty() {
        return Vec::new();
    }
    collect_top_level_lets(out, body)
        .into_iter()
        .filter(|d| {
            !d.has_type_annotation
                && !d.definite
                && d.init != DeclaratorInit::Other
                && is_target(target_names, &d.name)
        })
        .map(|d| (d.name_end, d.name))
        .collect()
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::is_runes_mode;

    fn runes(source: &str) -> bool {
        let (doc, _) = svn_parser::parse_sections(source);
        let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
        let alloc = oxc_allocator::Allocator::default();
        let instance = doc
            .instance_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        let module = doc
            .module_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        is_runes_mode(&doc, &fragment, instance.as_ref(), module.as_ref())
    }

    #[test]
    fn plain_rune_call_flips_runes_mode() {
        assert!(runes("<script>let x = $state(0);</script>"));
        assert!(runes("<script>let x = $derived.by(() => 1);</script>"));
    }

    #[test]
    fn member_call_rune_names_stay_legacy() {
        // A store lib method literally named `$state` is a property
        // access, not a rune — the component keeps its Svelte-4
        // default-export shape and events strictness.
        assert!(!runes(
            "<script>export let value; const s = api.$state(0);</script>"
        ));
        assert!(!runes("<script>this.$props();</script>"));
        assert!(!runes("<script>obj?.$state(0);</script>"));
        assert!(!runes("<script>const x = my$state(0);</script>"));
    }

    #[test]
    fn options_attr_forces_runes() {
        assert!(runes("<svelte:options runes /><script>let x = 1;</script>"));
    }

    #[test]
    fn rune_in_comment_string_or_markup_is_not_a_reference() {
        // Upstream reads runes mode from the script's unresolved
        // globals, so text that only looks like a rune call is ignored.
        assert!(!runes(
            "<script>// migrate to $state(0) later\nlet x = 1;</script>"
        ));
        assert!(!runes("<script>const s = \"$state(0)\";</script>"));
        assert!(!runes(
            "<script>let v = 1;</script><p>Call $state(0) to start</p>"
        ));
    }

    #[test]
    fn inspect_alone_does_not_flip_runes_mode() {
        assert!(!runes("<script>let v = 1; $inspect(v);</script>"));
    }

    #[test]
    fn props_rune_and_top_level_await_flip_runes_mode() {
        assert!(runes("<script>let { a } = $props();</script>"));
        assert!(runes("<script>const x = await fetch('/');</script>"));
        assert!(!runes(
            "<script>async function f() { await fetch('/'); }</script>"
        ));
    }

    #[test]
    fn template_expressions_count() {
        assert!(runes(
            "<script>let n = 1;</script>{#each [n] as x}{let y = $state(x)}{y}{/each}"
        ));
        assert!(runes(
            "<script>const p = Promise.resolve(1);</script><p>{await p}</p>"
        ));
        assert!(!runes(
            "<script>const f = async () => 1;</script><button onclick={async () => { await f(); }}>x</button>"
        ));
    }

    #[test]
    fn store_named_state_is_not_a_rune() {
        assert!(!runes(
            "<script>import { writable } from 'svelte/store'; const state = writable(0); $state.set(1);</script>"
        ));
    }

    #[test]
    fn marker_less_source_stays_legacy() {
        assert!(!runes(
            "<script>export let value: string; let total = 0;</script>\n<p>{value} {total}</p>"
        ));
    }
}
