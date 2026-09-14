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

use crate::process_instance_script_content;
use crate::sveltekit;
use crate::util::{is_ascii_ws, is_ident_byte};

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

/// SVELTE-4-COMPAT: heuristic detector for components that use Svelte-4
/// conventions, i.e. ones whose consumers are likely to pass `on:event`
/// directives (rewritten to `on<event>` prop keys), `slot="x"` named-slot
/// attrs, and similar Svelte-4-specific surface our Svelte-5 emit doesn't
/// model as declared props. Signals, any of which trips detection:
///
/// 1. Any `export let` declaration — strongest signal; Svelte 5 uses
///    `$props()` and `export { … }` instead.
/// 2. Any `<slot>` element in the template — Svelte 5 uses snippets.
/// 3. `createEventDispatcher` imported or called — Svelte 5 uses prop
///    callbacks instead of the dispatcher.
/// 4. `$$Props` / `$$Events` / `$$Slots` interface declared — explicit
///    Svelte-4 typing convention.
/// 5. `$$slots` / `$$props` / `$$restProps` ambients referenced.
///
/// False positives (a genuinely Svelte-5 file containing one of those
/// substrings in a comment) just add a widen clause that's structurally
/// a no-op against well-formed Svelte-5 consumer code. The reverse is
/// costlier — a missed Svelte-4 file surfaces hundreds of TS2353
/// "property does not exist" errors on every consumer.
pub(crate) fn is_svelte4_component(
    doc: &svn_parser::Document<'_>,
    split: Option<&process_instance_script_content::SplitScript>,
    has_slot: bool,
) -> bool {
    let instance_src = doc
        .instance_script
        .as_ref()
        .map(|s| s.content)
        .unwrap_or("");
    let module_src = doc.module_script.as_ref().map(|s| s.content).unwrap_or("");
    if contains_export_let(instance_src) || contains_export_let(module_src) {
        return true;
    }
    if has_slot {
        return true;
    }
    if instance_src.contains("createEventDispatcher")
        || module_src.contains("createEventDispatcher")
    {
        return true;
    }
    if has_double_dollar_interface(instance_src) || has_double_dollar_interface(module_src) {
        return true;
    }
    if doc.source.contains("$$slots")
        || doc.source.contains("$$restProps")
        || doc.source.contains("$$props")
    {
        return true;
    }
    if let Some(s) = split {
        if !s.exported_locals.is_empty() {
            return true;
        }
    }
    false
}

pub(crate) fn contains_export_let(src: &str) -> bool {
    // Loose: we want `export let` at word boundaries. Using substring is
    // too permissive (e.g. `/* export let X */` in a comment), but
    // comments would only false-positive, not false-negative — safe.
    let mut rest = src;
    while let Some(idx) = rest.find("export") {
        let after = &rest[idx + 6..];
        if let Some(non_ws) = after.find(|c: char| !c.is_whitespace()) {
            if after[non_ws..].starts_with("let") {
                let next = after[non_ws + 3..].chars().next();
                if matches!(next, Some(c) if c.is_whitespace() || c == '/') {
                    return true;
                }
            }
        }
        rest = &rest[idx + 6..];
    }
    false
}

fn has_double_dollar_interface(src: &str) -> bool {
    src.contains("interface $$Props")
        || src.contains("interface $$Events")
        || src.contains("interface $$Slots")
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

/// Infer Svelte 5 runes mode from the document source.
///
/// Deliberately NOT comment/string-aware, unlike svn_core's
/// [`svn_core::rune_scan::script_calls_rune`] (svn-lint's scan): a
/// `$state(0)` inside a comment or string literal DOES flip emit's
/// runes mode. The two scans share the marker-anchoring primitive
/// (`find_marker_from`) but keep their distinct match semantics.
///
/// Emit only ever sees `.svelte` documents, so runes mode is inferred
/// purely from rune-call markers in the source — there is no filename
/// signal to consult here. The `.svelte.js` / `.svelte.ts` filename
/// case is svn-lint's concern for the module files emit never
/// transforms.
///
/// Looks for any rune call (`$state(…)`, `$props(…)`, `$derived(…)`,
/// `$effect(…)`, `$bindable(…)`, `$inspect(…)`, `$host(…)`). Runes are
/// always called, so requiring `(` after the name excludes the ambient
/// `$$props` store pattern cheaply. Dotted variants (`$state.raw`,
/// `$derived.by`) are matched by walking past the `.word` chain before
/// the `(`.
pub(crate) fn is_runes_mode(
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
) -> bool {
    // An explicit `<svelte:options runes>` / `runes={true}` forces runes
    // ON — mirroring upstream svelte2tsx `ExportedNames.isRunesMode`,
    // which OR-s the option in. `runes={false}` cannot force runes OFF
    // here (the OR still yields runes when a `$props()`/`$state()` call
    // is present), so only `Some(true)` participates.
    if svn_parser::runes_option(fragment, doc.source) == Some(true) {
        return true;
    }
    let bytes = doc.source.as_bytes();
    let mut i = 0;
    while let Some((pos, marker_len)) = svn_core::rune_scan::find_marker_from(bytes, i) {
        i = pos + 1;
        // Identifier tails (`$$props`, `my$state`) and member-access
        // property names (`api.$state(0)`, `this?.$props()`) are not
        // rune usages — shared guard with svn_core's rune scan.
        if svn_core::rune_scan::rune_marker_is_shadowed_at(bytes, pos) {
            continue;
        }
        let mut after = pos + marker_len;
        while bytes.get(after) == Some(&b'.') {
            after += 1;
            while after < bytes.len()
                && (bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_')
            {
                after += 1;
            }
        }
        while after < bytes.len() && matches!(bytes[after], b' ' | b'\t') {
            after += 1;
        }
        if bytes.get(after) == Some(&b'(') {
            return true;
        }
    }
    false
}

/// SVELTE-4-COMPAT: de-narrow a typed-with-initializer exported
/// declaration. Scans `out` for `let NAME: T = EXPR;` where NAME is
/// in `target_names`, then inserts `NAME = undefined as any;`
/// immediately after the terminating `;`. The cast widens TS's
/// flow-narrowed type back to the declared annotation, so later
/// comparisons like `NAME === 'other-literal'` don't fire TS2367.
///
/// Declarations without a type annotation or without an initializer
/// are skipped — those are already handled by the widen / definite-
/// assign passes.
/// SVELTE-4-COMPAT: rewrite every parenthesized sequence expression
/// emitted in two specific Svelte-4 idiomatic positions to an array
/// literal:
///
///   1. `void (a, b, c)`     → `void [a, b, c]`
///   2. `$: (a, b, c)`       → `$: void [a, b, c]`
///
/// Both shapes are how Svelte-4 components declare reactive
/// dependencies: `$:` re-runs whenever any referenced identifier
/// changes, and `void (deps…)` is the canonical "list deps without
/// using them" pattern when the actual side effect is in a separate
/// statement. tsgo's strict checking fires TS2871 ("Left side of
/// comma operator is unused and has no side effects") on every comma
/// in the list because each LHS of a comma is just an identifier
/// read with no side effect. The array-literal form puts each
/// reference in array-element position where TS treats it as "used"
/// and the warning doesn't fire. Runtime semantics are equivalent
/// (both forms evaluate every expression and discard the result),
/// and the rewrite lives entirely in our type-check overlay so user
/// runtime behaviour is untouched.
///
/// Detection: `void` or `$:` keyword followed (after ws/newlines) by
/// `(`, scan paren-balanced content; rewrite ONLY if a top-level `,`
/// was seen (i.e. a sequence expression, not just `void (x)` with a
/// single expression in parens). String / template / comment content
/// inside the parens is skipped so a `,` inside a string doesn't
/// trigger a false rewrite.
///
/// Returns the insertions performed (the `void ` prefix on `$:` labels
/// — the paren→bracket swaps are length-preserving and need no
/// re-anchoring) as ascending `(position, length)` pairs in PRE-rewrite
/// buffer coordinates, same contract as
/// [`rewrite_definite_assignment_in_place`].
pub(crate) fn rewrite_void_sequence_to_array(out: &mut String) -> Vec<(u32, u32)> {
    // Phase 1: scan for sequence sites without touching the buffer —
    // most files have none and return with zero allocation.
    let bytes = out.as_bytes();
    let mut sites: Vec<(SeqKind, usize, usize)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let c = bytes[i];
        // Skip string / template literals and comments so a
        // `void (a, b)` appearing inside `"…"` or `// …` isn't rewritten.
        if matches!(c, b'"' | b'\'' | b'`') {
            i = skip_string_literal(bytes, i, c);
            continue;
        }
        if c == b'/' && bytes.get(i + 1).copied() == Some(b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(i + 1).copied() == Some(b'*') {
            let mut k = i + 2;
            while k + 1 < bytes.len() && !(bytes[k] == b'*' && bytes[k + 1] == b'/') {
                k += 1;
            }
            i = k.saturating_add(2).min(bytes.len());
            continue;
        }
        if let Some(site) = find_paren_sequence(bytes, i) {
            i = site.2 + 1;
            sites.push(site);
        } else {
            i += 1;
        }
    }
    let mut edits: Vec<(u32, u32)> = Vec::new();
    if sites.is_empty() {
        return edits;
    }
    // Phase 2: rebuild once with bulk segment copies. The paren →
    // bracket swaps are length-preserving; only the `void ` prefix on
    // `$:` labels grows the buffer.
    let original = std::mem::take(out);
    let mut rebuilt = String::with_capacity(original.len() + sites.len() * 5);
    let mut cursor = 0;
    for (kind, paren_open, paren_close) in sites {
        rebuilt.push_str(&original[cursor..paren_open]);
        if matches!(kind, SeqKind::ReactiveLabel) {
            rebuilt.push_str("void ");
            edits.push((paren_open as u32, 5));
        }
        rebuilt.push('[');
        rebuilt.push_str(&original[paren_open + 1..paren_close]);
        rebuilt.push(']');
        cursor = paren_close + 1;
    }
    rebuilt.push_str(&original[cursor..]);
    *out = rebuilt;
    edits
}

#[derive(Copy, Clone)]
enum SeqKind {
    /// `void (a, b)` — already has `void`, just swap parens for brackets.
    Void,
    /// `$: (a, b)` — emit `void` prefix in addition to swapping parens.
    ReactiveLabel,
}

/// At byte `i`, look for `void` or `$:` followed by a parenthesized
/// sequence expression. Returns `(kind, paren_open, paren_close)` iff
/// matched. Match conditions:
///   - `void` keyword: byte[i..i+4] == `void` AND surrounding
///     boundaries are not identifier chars (otherwise `avoid` /
///     `void_x` would match), then ws, then `(`
///   - `$:` label: byte[i..i+2] == `$:` AND not preceded by an
///     identifier char (avoids `foo$:bar` though that's not valid
///     JS anyway), then ws, then `(`
///   - inside the parens, at least one TOP-LEVEL `,` (paren_depth=1)
fn find_paren_sequence(bytes: &[u8], i: usize) -> Option<(SeqKind, usize, usize)> {
    let (kind, after_keyword) = if i + 4 <= bytes.len()
        && &bytes[i..i + 4] == b"void"
        && (i == 0 || !is_ident_byte(bytes[i - 1]))
        && bytes.get(i + 4).copied().is_some_and(|b| !is_ident_byte(b))
    {
        (SeqKind::Void, i + 4)
    } else if i + 2 <= bytes.len()
        && &bytes[i..i + 2] == b"$:"
        && (i == 0 || !is_ident_byte(bytes[i - 1]))
    {
        (SeqKind::ReactiveLabel, i + 2)
    } else {
        return None;
    };
    let mut p = after_keyword;
    while p < bytes.len() && is_ascii_ws(bytes[p]) {
        p += 1;
    }
    if p >= bytes.len() || bytes[p] != b'(' {
        return None;
    }
    let paren_open = p;
    let mut depth: i32 = 1;
    let mut top_level_comma = false;
    let mut s = paren_open + 1;
    while s < bytes.len() && depth > 0 {
        let c = bytes[s];
        match c {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth -= 1;
                if depth == 0 {
                    if !top_level_comma {
                        return None;
                    }
                    return Some((kind, paren_open, s));
                }
            }
            b',' if depth == 1 => {
                top_level_comma = true;
            }
            b'"' | b'\'' | b'`' => {
                s = skip_string_literal(bytes, s, c);
                continue;
            }
            b'/' if bytes.get(s + 1).copied() == Some(b'/') => {
                while s < bytes.len() && bytes[s] != b'\n' {
                    s += 1;
                }
                continue;
            }
            b'/' if bytes.get(s + 1).copied() == Some(b'*') => {
                let mut k = s + 2;
                while k + 1 < bytes.len() && !(bytes[k] == b'*' && bytes[k + 1] == b'/') {
                    k += 1;
                }
                s = k.saturating_add(2).min(bytes.len());
                continue;
            }
            _ => {}
        }
        s += 1;
    }
    None
}

/// Skip past a JS string / template literal starting at `start`
/// (which is the opening quote). Returns the byte position AFTER
/// the closing quote. Handles `\\` escapes and template
/// `${ … }` interpolations recursively (depth tracked so a `}`
/// inside an interpolation expression doesn't close the template).
fn skip_string_literal(bytes: &[u8], start: usize, quote: u8) -> usize {
    let mut s = start + 1;
    while s < bytes.len() {
        let c = bytes[s];
        if c == b'\\' {
            s += 2;
            continue;
        }
        if c == quote {
            return s + 1;
        }
        if quote == b'`' && c == b'$' && bytes.get(s + 1).copied() == Some(b'{') {
            let mut depth = 1;
            s += 2;
            while s < bytes.len() && depth > 0 {
                match bytes[s] {
                    b'{' => depth += 1,
                    b'}' => depth -= 1,
                    b'"' | b'\'' | b'`' => {
                        s = skip_string_literal(bytes, s, bytes[s]);
                        continue;
                    }
                    _ => {}
                }
                s += 1;
            }
            continue;
        }
        s += 1;
    }
    s
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
/// $$restProps = …;` at the top of the render function when the source
/// references them.
///
/// Substring detection is deliberately loose — we don't parse to see
/// whether the occurrence is a real identifier vs. string content. A
/// spurious injection is harmless (the declared local just goes
/// unused), whereas a missed one fires TS2304 across every reference
/// and cascades through the surrounding expression's typing.
///
/// Types: `Record<string, any>` for all three. Upstream's
/// `__sveltets_2_slotsType({…slot names…})` is more precise (each
/// slot is typed as `boolean | ''`), but that requires walking the
/// template to collect slot names and emit a shape literal. We'll do
/// that in Phase 2 if the loose ambient isn't sufficient.
pub(crate) fn emit_svelte4_ambients(out: &mut String, doc: &svn_parser::Document<'_>, is_ts: bool) {
    let src = doc.source;
    // In TS overlays we emit inline `: T` annotations. In JS overlays
    // we must not — tsgo fires TS8010 and aborts project-wide once
    // hit, silently suppressing every legitimate diagnostic
    // elsewhere. Emit JSDoc casts on the RHS for JS overlays.
    if src.contains("$$slots") {
        if is_ts {
            out.push_str("    let $$slots: Record<string, boolean | undefined> = {};\n");
        } else {
            out.push_str(
                "    let $$slots = /** @type {Record<string, boolean | undefined>} */ ({});\n",
            );
        }
        out.push_str("    void $$slots;\n");
    }
    if src.contains("$$restProps") {
        if is_ts {
            out.push_str("    let $$restProps: Record<string, any> = {};\n");
        } else {
            out.push_str("    let $$restProps = /** @type {Record<string, any>} */ ({});\n");
        }
        out.push_str("    void $$restProps;\n");
    }
    // `$$props` detection is intentionally loose substring matching,
    // consistent with the `$$slots` / `$$restProps` branches above. A
    // spurious match just declares an unused local, which is harmless;
    // a missed one would fire TS2304 across every reference.
    if src.contains("$$props") {
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
        is_runes_mode(&doc, &fragment)
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
    fn rune_in_comment_or_string_still_flips_runes_mode() {
        // Emit's runes detection is deliberately comment/string-BLIND —
        // a rune call mentioned in a comment or string literal counts.
        // svn_core::rune_scan::script_calls_rune (svn-lint's scan) is
        // the comment-aware variant; the two must not be unified.
        assert!(runes(
            "<script>// migrate to $state(0) later\nlet x = 1;</script>"
        ));
        assert!(runes("<script>const s = \"$state(0)\";</script>"));
    }

    #[test]
    fn marker_less_source_stays_legacy() {
        assert!(!runes(
            "<script>export let value: string; let total = 0;</script>\n<p>{value} {total}</p>"
        ));
    }
}
