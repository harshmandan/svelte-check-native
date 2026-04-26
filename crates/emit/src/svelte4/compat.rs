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

use smol_str::SmolStr;

use crate::process_instance_script_content;
use crate::sveltekit;
use crate::util::{is_ascii_ws, is_ident_byte, utf8_char_len};

/// Rewrite `let <name>: T;` → `let <name>!: T;` for each bind-target
/// identifier, in-place inside the already-built output buffer.
///
/// Svelte assigns a `bind:this` target asynchronously, after the binding
/// element mounts. TypeScript's flow analysis can't see that, so any
/// closure reading the variable would be flagged "used before being
/// assigned" (TS2454). The `!:` definite-assignment assertion tells
/// TypeScript to trust us.
///
/// Only matches `let <name>:` patterns with the colon — declarations with
/// initializers (`let x = ...`) are already definitely assigned and don't
/// need the `!`. Adding `!` to an initialized declaration is itself a TS
/// error (TS1263).
pub(crate) fn rewrite_definite_assignment_in_place(out: &mut String, target_names: &[SmolStr]) {
    if target_names.is_empty() {
        return;
    }
    let original = std::mem::take(out);
    let bytes = original.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((stmt_end, insertions)) = try_process_let_statement(bytes, i, target_names) {
            // Emit the let statement with `!` inserted after each matched
            // binding identifier. Insertions are in ascending order by
            // byte position — emit segments between them verbatim.
            let mut cursor = i;
            for pos in &insertions {
                out.push_str(&original[cursor..*pos]);
                out.push('!');
                cursor = *pos;
            }
            out.push_str(&original[cursor..stmt_end]);
            i = stmt_end;
        } else {
            let ch_len = utf8_char_len(bytes[i]);
            out.push_str(&original[i..i + ch_len]);
            i += ch_len;
        }
    }
}

/// At byte position `i`, try to recognize an entire `let …;` statement
/// and collect every binding identifier that needs a `!` definite-
/// assignment assertion. Returns `Some((stmt_end, insertions))` on
/// success, where `insertions` are byte positions (ascending) at which
/// to splice `!`. `stmt_end` is the byte position AFTER the terminating
/// `;` (or at newline / EOF if no `;`).
///
/// A declarator qualifies for insertion when its binding is a target
/// name AND it has a `:` type annotation AND has NO `=` initializer
/// before the next `,` or `;` (per TS1263: definite-assignment
/// assertions are illegal with initializers).
///
/// Handles both the simple shape `let foo: T;` and the multi-
/// declarator shape `let a: A = v, b: B, c: C = v;` — each declarator
/// is checked independently.
fn try_process_let_statement(
    bytes: &[u8],
    i: usize,
    target_names: &[SmolStr],
) -> Option<(usize, Vec<usize>)> {
    if i + 3 > bytes.len() || &bytes[i..i + 3] != b"let" {
        return None;
    }
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return None;
    }
    let after_let = i + 3;
    if after_let >= bytes.len() || !is_ascii_ws(bytes[after_let]) {
        return None;
    }

    let mut insertions: Vec<usize> = Vec::new();
    let mut p = after_let;
    loop {
        while p < bytes.len() && is_ascii_ws(bytes[p]) {
            p += 1;
        }
        if p >= bytes.len()
            || !(bytes[p].is_ascii_alphabetic() || bytes[p] == b'_' || bytes[p] == b'$')
        {
            return None;
        }
        let name_start = p;
        while p < bytes.len() && is_ident_byte(bytes[p]) {
            p += 1;
        }
        let name_end = p;
        let name = &bytes[name_start..name_end];

        let mut s = name_end;
        while s < bytes.len() && is_ascii_ws(bytes[s]) {
            s += 1;
        }
        let has_type_annotation = s < bytes.len() && bytes[s] == b':';
        if has_type_annotation {
            s += 1;
        }

        let mut has_initializer = false;
        let mut paren_depth: i32 = 0;
        while s < bytes.len() {
            let c = bytes[s];
            match c {
                b'(' | b'[' | b'{' | b'<' => paren_depth += 1,
                b')' | b']' | b'}' => paren_depth -= 1,
                b'>' => {
                    // Two non-generic-close forms to guard against:
                    //   `=>` — arrow return (prev byte is `=`)
                    //   `>=` — greater-or-equal (next byte is `=`).
                    let prev = if s > 0 { Some(bytes[s - 1]) } else { None };
                    let next = bytes.get(s + 1).copied();
                    let skip_decrement = prev == Some(b'=') || next == Some(b'=');
                    if !skip_decrement {
                        paren_depth -= 1;
                    }
                }
                b'=' if paren_depth == 0 => {
                    let next = bytes.get(s + 1).copied();
                    match next {
                        Some(b'>') => {
                            s += 2;
                            continue;
                        }
                        Some(b'=') => {
                            s += 1;
                        }
                        _ => {
                            has_initializer = true;
                        }
                    }
                }
                b',' | b';' if paren_depth == 0 => break,
                b'\n' if paren_depth == 0 => {
                    // ASI at a statement boundary — but only when the
                    // next non-whitespace character doesn't continue
                    // a type annotation. Multi-line union/
                    // intersection types continue across newlines
                    // with a leading `|` / `&`.
                    let mut k = s + 1;
                    while k < bytes.len() && matches!(bytes[k], b' ' | b'\t') {
                        k += 1;
                    }
                    let next_nonws = bytes.get(k).copied();
                    if !matches!(next_nonws, Some(b'|') | Some(b'&')) {
                        break;
                    }
                }
                _ => {}
            }
            s += 1;
        }

        if has_type_annotation
            && !has_initializer
            && target_names.iter().any(|t| t.as_bytes() == name)
        {
            insertions.push(name_end);
        }

        if s >= bytes.len() {
            return Some((s, insertions));
        }
        match bytes[s] {
            b',' => {
                p = s + 1;
                continue;
            }
            b';' => {
                return Some((s + 1, insertions));
            }
            b'\n' => {
                return Some((s, insertions));
            }
            _ => {
                return Some((s, insertions));
            }
        }
    }
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
/// script. When true, the default-export declaration intersects with
/// `& { readonly __svn_events: $$Events }` so consumers resolve to
/// `__svn_ensure_component`'s typed overload and get narrowed
/// `$on("evt", handler)` signatures.
pub(crate) fn has_strict_events(doc: &svn_parser::Document<'_>) -> bool {
    let instance_src = doc
        .instance_script
        .as_ref()
        .map(|s| s.content)
        .unwrap_or("");
    let module_src = doc.module_script.as_ref().map(|s| s.content).unwrap_or("");
    let has_interface =
        |src: &str| src.contains("interface $$Events") || src.contains("type $$Events ");
    has_interface(instance_src) || has_interface(module_src)
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

/// Infer Svelte 5 runes mode from the document source. Mirrors
/// `svn_lint::walk::infer_runes_mode` (intentionally duplicated rather
/// than dep'ing on lint — emit needs the signal with no circular dep
/// in the other direction).
///
/// Looks for any rune call (`$state(…)`, `$props(…)`, `$derived(…)`,
/// `$effect(…)`, `$bindable(…)`, `$inspect(…)`, `$host(…)`) or a
/// `.svelte.js` / `.svelte.ts` filename. Runes are always called, so
/// requiring `(` after the name excludes the ambient `$$props` store
/// pattern cheaply. Dotted variants (`$state.raw`, `$derived.by`) are
/// matched by walking past the `.word` chain before the `(`.
pub(crate) fn is_runes_mode(doc: &svn_parser::Document<'_>) -> bool {
    let source = doc.source;
    for marker in [
        "$state",
        "$derived",
        "$effect",
        "$props",
        "$bindable",
        "$inspect",
        "$host",
    ] {
        let bytes = source.as_bytes();
        let mbytes = marker.as_bytes();
        let mut i = 0;
        while let Some(rel) = bytes[i..].windows(mbytes.len()).position(|w| w == mbytes) {
            let pos = i + rel;
            if pos.checked_sub(1).and_then(|p| bytes.get(p)).copied() == Some(b'$') {
                i = pos + mbytes.len();
                continue;
            }
            let mut after = pos + mbytes.len();
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
            i = pos + mbytes.len();
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
pub(crate) fn denarrow_typed_exported_props_in_place(out: &mut String, target_names: &[SmolStr]) {
    if target_names.is_empty() {
        return;
    }
    let original = std::mem::take(out);
    let bytes = original.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((stmt_end, matched_names)) =
            try_process_let_statement_for_denarrow(bytes, i, target_names)
        {
            out.push_str(&original[i..stmt_end]);
            if !matched_names.is_empty() {
                if !out.ends_with(';') {
                    out.push(';');
                }
                for name in &matched_names {
                    out.push(' ');
                    out.push_str(name);
                    out.push_str(" = undefined as any;");
                }
            }
            i = stmt_end;
        } else {
            let ch_len = utf8_char_len(bytes[i]);
            out.push_str(&original[i..i + ch_len]);
            i += ch_len;
        }
    }
}

/// Declarator scanner matched to `try_process_let_statement_*` twins.
/// Qualifies a declarator when: name IS a target AND has a `:` type
/// annotation AND has an `=` initializer. Returns the matched names
/// so the caller can append assignments after the statement.
fn try_process_let_statement_for_denarrow(
    bytes: &[u8],
    i: usize,
    target_names: &[SmolStr],
) -> Option<(usize, Vec<SmolStr>)> {
    if i + 3 > bytes.len() || &bytes[i..i + 3] != b"let" {
        return None;
    }
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return None;
    }
    let after_let = i + 3;
    if after_let >= bytes.len() || !is_ascii_ws(bytes[after_let]) {
        return None;
    }

    let mut matched: Vec<SmolStr> = Vec::new();
    let mut p = after_let;
    loop {
        while p < bytes.len() && is_ascii_ws(bytes[p]) {
            p += 1;
        }
        if p >= bytes.len()
            || !(bytes[p].is_ascii_alphabetic() || bytes[p] == b'_' || bytes[p] == b'$')
        {
            return None;
        }
        let name_start = p;
        while p < bytes.len() && is_ident_byte(bytes[p]) {
            p += 1;
        }
        let name_end = p;
        let name_bytes = &bytes[name_start..name_end];

        let mut s = name_end;
        while s < bytes.len() && is_ascii_ws(bytes[s]) {
            s += 1;
        }
        if s < bytes.len() && bytes[s] == b'!' {
            s += 1;
            while s < bytes.len() && is_ascii_ws(bytes[s]) {
                s += 1;
            }
        }
        let has_type_annotation = s < bytes.len() && bytes[s] == b':';
        if has_type_annotation {
            s += 1;
        }

        let mut has_initializer = false;
        let mut paren_depth: i32 = 0;
        while s < bytes.len() {
            let c = bytes[s];
            match c {
                b'(' | b'[' | b'{' | b'<' => paren_depth += 1,
                b')' | b']' | b'}' => paren_depth -= 1,
                b'>' => {
                    let prev = if s > 0 { Some(bytes[s - 1]) } else { None };
                    if prev != Some(b'=') {
                        paren_depth -= 1;
                    }
                }
                b'=' if paren_depth == 0 => {
                    let next = bytes.get(s + 1).copied();
                    match next {
                        Some(b'>') => {
                            s += 2;
                            continue;
                        }
                        Some(b'=') => {
                            s += 1;
                        }
                        _ => {
                            has_initializer = true;
                        }
                    }
                }
                b',' | b';' if paren_depth == 0 => break,
                b'\n' if paren_depth == 0 => {
                    let mut prev = s;
                    while prev > 0 {
                        prev -= 1;
                        let b = bytes[prev];
                        if b != b' ' && b != b'\t' {
                            break;
                        }
                    }
                    let prev_continues = bytes[prev] == b'=';
                    let mut next = s + 1;
                    while next < bytes.len()
                        && (bytes[next] == b' ' || bytes[next] == b'\t' || bytes[next] == b'\n')
                    {
                        next += 1;
                    }
                    let next_continues = next < bytes.len()
                        && matches!(
                            bytes[next],
                            b'?' | b':'
                                | b'.'
                                | b'&'
                                | b'|'
                                | b'+'
                                | b'-'
                                | b'*'
                                | b'/'
                                | b'%'
                                | b'^'
                                | b'='
                        );
                    if !prev_continues && !next_continues {
                        break;
                    }
                }
                _ => {}
            }
            s += 1;
        }

        if has_initializer && target_names.iter().any(|t| t.as_bytes() == name_bytes) {
            if let Ok(name_str) = std::str::from_utf8(name_bytes) {
                matched.push(SmolStr::from(name_str));
            }
        }

        if s >= bytes.len() {
            return Some((s, matched));
        }
        match bytes[s] {
            b',' => {
                p = s + 1;
                continue;
            }
            b';' => return Some((s + 1, matched)),
            b'\n' => return Some((s, matched)),
            _ => return Some((s, matched)),
        }
    }
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
    // `$$props` detection has to avoid `$$restProps` — the word
    // boundary check: must not be preceded by `rest` (the only
    // Svelte-4 form where $$props appears as a suffix).
    if let Some(idx) = src.find("$$props") {
        let prev = src.as_bytes().get(idx.saturating_sub(4)..idx);
        let is_rest = matches!(prev, Some(b"rest"));
        if !is_rest || src.matches("$$props").count() > src.matches("$$restProps").count() {
            if is_ts {
                out.push_str("    let $$props: Record<string, any> = {};\n");
            } else {
                out.push_str("    let $$props = /** @type {Record<string, any>} */ ({});\n");
            }
            out.push_str("    void $$props;\n");
        }
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
    target_names: &[SmolStr],
    route_kind: Option<sveltekit::RouteKind>,
) {
    if target_names.is_empty() {
        return;
    }
    let original = std::mem::take(out);
    let bytes = original.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((stmt_end, insertions)) =
            try_process_let_statement_for_widening(bytes, i, target_names)
        {
            let mut cursor = i;
            for (pos, name) in &insertions {
                out.push_str(&original[cursor..*pos]);
                let kit_type = route_kind.and_then(|k| sveltekit::kit_widen_type(name, k));
                match kit_type {
                    Some(ty) => {
                        out.push_str(" = /** @type {");
                        out.push_str(ty);
                        out.push_str("} */ (/** @type {any} */ (null))");
                    }
                    None => {
                        out.push_str(" = /** @type {any} */ (null)");
                    }
                }
                cursor = *pos;
            }
            out.push_str(&original[cursor..stmt_end]);
            i = stmt_end;
        } else {
            let ch_len = utf8_char_len(bytes[i]);
            out.push_str(&original[i..i + ch_len]);
            i += ch_len;
        }
    }
}

pub(crate) fn widen_untyped_exported_props_in_place(
    out: &mut String,
    target_names: &[SmolStr],
    route_kind: Option<sveltekit::RouteKind>,
) {
    if target_names.is_empty() {
        return;
    }
    let original = std::mem::take(out);
    let bytes = original.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if let Some((stmt_end, insertions)) =
            try_process_let_statement_for_widening(bytes, i, target_names)
        {
            let mut cursor = i;
            for (pos, name) in &insertions {
                out.push_str(&original[cursor..*pos]);
                let widen_type = route_kind
                    .and_then(|k| sveltekit::kit_widen_type(name, k))
                    .unwrap_or("any");
                out.push_str(": ");
                out.push_str(widen_type);
                cursor = *pos;
            }
            out.push_str(&original[cursor..stmt_end]);
            i = stmt_end;
        } else {
            let ch_len = utf8_char_len(bytes[i]);
            out.push_str(&original[i..i + ch_len]);
            i += ch_len;
        }
    }
}

/// Declarator scanner twin of `try_process_let_statement`. The
/// traversal is byte-for-byte identical; only the qualification rule
/// flips: widen when name IS a target AND has NO type AND NO
/// initializer.
fn try_process_let_statement_for_widening(
    bytes: &[u8],
    i: usize,
    target_names: &[SmolStr],
) -> Option<(usize, Vec<(usize, SmolStr)>)> {
    if i + 3 > bytes.len() || &bytes[i..i + 3] != b"let" {
        return None;
    }
    if i > 0 && is_ident_byte(bytes[i - 1]) {
        return None;
    }
    let after_let = i + 3;
    if after_let >= bytes.len() || !is_ascii_ws(bytes[after_let]) {
        return None;
    }

    let mut insertions: Vec<(usize, SmolStr)> = Vec::new();
    let mut p = after_let;
    loop {
        while p < bytes.len() && is_ascii_ws(bytes[p]) {
            p += 1;
        }
        if p >= bytes.len()
            || !(bytes[p].is_ascii_alphabetic() || bytes[p] == b'_' || bytes[p] == b'$')
        {
            return None;
        }
        let name_start = p;
        while p < bytes.len() && is_ident_byte(bytes[p]) {
            p += 1;
        }
        let name_end = p;
        let name = &bytes[name_start..name_end];

        let mut s = name_end;
        while s < bytes.len() && is_ascii_ws(bytes[s]) {
            s += 1;
        }
        let has_type_annotation = s < bytes.len() && bytes[s] == b':';
        if has_type_annotation {
            s += 1;
        }

        let mut has_initializer = false;
        let mut initializer_is_nullish = false;
        let mut paren_depth: i32 = 0;
        while s < bytes.len() {
            let c = bytes[s];
            match c {
                b'(' | b'[' | b'{' | b'<' => paren_depth += 1,
                b')' | b']' | b'}' => paren_depth -= 1,
                b'>' => {
                    let prev = if s > 0 { Some(bytes[s - 1]) } else { None };
                    if prev != Some(b'=') {
                        paren_depth -= 1;
                    }
                }
                b'=' if paren_depth == 0 => {
                    let next = bytes.get(s + 1).copied();
                    match next {
                        Some(b'>') => {
                            s += 2;
                            continue;
                        }
                        Some(b'=') => {
                            s += 1;
                        }
                        _ => {
                            has_initializer = true;
                            // Peek past whitespace for a literal
                            // `undefined` / `null` initializer.
                            let mut p2 = s + 1;
                            while p2 < bytes.len() && is_ascii_ws(bytes[p2]) {
                                p2 += 1;
                            }
                            let rest = &bytes[p2..];
                            let is_word_end = |off: usize| {
                                bytes
                                    .get(p2 + off)
                                    .copied()
                                    .map(|b| !is_ident_byte(b))
                                    .unwrap_or(true)
                            };
                            if (rest.starts_with(b"undefined") && is_word_end(9))
                                || (rest.starts_with(b"null") && is_word_end(4))
                            {
                                initializer_is_nullish = true;
                            }
                        }
                    }
                }
                b',' | b';' | b'\n' if paren_depth == 0 => break,
                _ => {}
            }
            s += 1;
        }

        let should_widen = !has_type_annotation
            && (!has_initializer || initializer_is_nullish)
            && target_names.iter().any(|t| t.as_bytes() == name);
        if should_widen {
            let name_str = std::str::from_utf8(name).ok().map(SmolStr::from)?;
            insertions.push((name_end, name_str));
        }

        if s >= bytes.len() {
            return Some((s, insertions));
        }
        match bytes[s] {
            b',' => {
                p = s + 1;
                continue;
            }
            b';' => return Some((s + 1, insertions)),
            b'\n' => return Some((s, insertions)),
            _ => return Some((s, insertions)),
        }
    }
}
