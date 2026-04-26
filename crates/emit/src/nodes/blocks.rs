//! Block-statement emitters: `{#each}`, `{#await}` then-branch,
//! `{#snippet}`, plus the `{:then}` / `{:catch}` branch helper used by
//! await blocks.
//!
//! Each emitter writes a TS-equivalent for-of / arrow / lexical-scope
//! wrapper that gives the user-introduced bindings a real lexical
//! introduction so type-checking inside the block body works.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::{EachBlock, Fragment, SnippetBlock};

use crate::emit_buffer::EmitBuffer;
use crate::nodes::component::annotate_snippet_params;
use crate::{all_identifiers, emit_is_ts, emit_template_body};

/// Emit a `for`-of loop for an `{#each}` block.
///
/// `{#each items}` without an `as` clause is legal Svelte (iterate N times,
/// discard the value); we use `__svn_each_unused` as a placeholder binding
/// so the emitted TypeScript stays syntactically valid.
pub(crate) fn emit_each_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &EachBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let raw_expr = source
        .get(b.expression_range.start as usize..b.expression_range.end as usize)
        .unwrap_or("undefined");
    let expr_text = raw_expr.trim();
    // Trimmed-slice source range so tsgo diagnostics fired anywhere
    // inside the expression (e.g. TS18048 on `.sort((a, b) => …)`
    // callback params) map back to their actual user-source byte
    // positions via the token map. Without this, callback-param
    // diagnostics fall in a synthesized region (the `__svn_each_items(...)`
    // wrapper span) and `map_diagnostic` drops them.
    let expr_source_range: Option<svn_core::Range> = if expr_text.is_empty() {
        None
    } else {
        let leading_ws = (raw_expr.len() - raw_expr.trim_start().len()) as u32;
        let start = b.expression_range.start + leading_ws;
        Some(svn_core::Range::new(start, start + expr_text.len() as u32))
    };
    let binding_text = match &b.as_clause {
        Some(c) => source
            .get(c.context_range.start as usize..c.context_range.end as usize)
            .unwrap_or("__svn_each_unused")
            .to_string(),
        None => "__svn_each_unused".to_string(),
    };
    // `{#each items as item, i}` — `i` is the zero-based iteration index.
    // `__svn_each_items` returns a plain Iterable, which doesn't expose
    // `.entries()`, so declaring the index as a `const i: number` inside
    // the loop body is simpler than rewriting the for-of pattern to
    // destructure an [index, item] pair. Using `0` for the value is a
    // type-only trick — the generated function is never executed.
    let index_binding: Option<&str> = b
        .as_clause
        .as_ref()
        .and_then(|c| c.index_range)
        .and_then(|r| source.get(r.start as usize..r.end as usize));
    let _ = write!(
        buf,
        "{indent}for (const {binding_text} of __svn_each_items("
    );
    match expr_source_range {
        Some(r) => buf.append_with_source(expr_text, r),
        None => buf.push_str(expr_text),
    }
    let _ = writeln!(buf, ")) {{");
    if let Some(ix) = index_binding {
        if emit_is_ts() {
            let _ = writeln!(buf, "{indent}    const {ix}: number = 0;");
        } else {
            let _ = writeln!(buf, "{indent}    /** @type {{number}} */ const {ix} = 0;");
        }
    }
    emit_template_body(buf, source, &b.body, depth + 1, insts, action_counter);
    // Void every identifier that the binding pattern destructures, not
    // just the first. `[id, label]` and `[id, { label }]` both bind two
    // names and TS6133 fires on each unused one.
    for ident in all_identifiers(&binding_text) {
        let _ = writeln!(buf, "{indent}    void {ident};");
    }
    if let Some(ix) = index_binding {
        let _ = writeln!(buf, "{indent}    void {ix};");
    }
    let _ = writeln!(buf, "{indent}}}");

    if let Some(alt) = &b.alternate {
        emit_template_body(buf, source, alt, depth, insts, action_counter);
    }
}

/// Walk `{:then v}` / `{:catch e}` body in a fresh lexical scope that
/// declares the branch's context binding as `any`. Without the
/// declaration, references to the bound name inside the branch (a
/// component-prop check that passes `v` as a prop, a subsequent
/// `{#each v as item}`) fire TS2304.
///
/// Supports destructure patterns (`{:then { a, b }}`, `{:then [x]}`)
/// via `all_identifiers`. An absent context range (`{:then}` with no
/// binding) skips the scope and just walks the body inline.
pub(crate) fn emit_branch_with_binding(
    buf: &mut EmitBuffer,
    source: &str,
    context_range: Option<&svn_core::Range>,
    body: &Fragment,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let Some(range) = context_range else {
        emit_template_body(buf, source, body, depth, insts, action_counter);
        return;
    };
    let binding_text = source
        .get(range.start as usize..range.end as usize)
        .unwrap_or("")
        .trim();
    if binding_text.is_empty() {
        emit_template_body(buf, source, body, depth, insts, action_counter);
        return;
    }
    let idents = all_identifiers(binding_text);
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}{{");
    let is_ts = emit_is_ts();
    for ident in &idents {
        if is_ts {
            let _ = writeln!(buf, "{indent}    const {ident}: any = undefined;");
        } else {
            let _ = writeln!(buf, "{indent}    const {ident} = undefined;");
        }
    }
    emit_template_body(buf, source, body, depth + 1, insts, action_counter);
    for ident in &idents {
        let _ = writeln!(buf, "{indent}    void {ident};");
    }
    let _ = writeln!(buf, "{indent}}}");
}

/// Emit `{:then v}` branch with upstream's await-binding shape:
///     ;async () => {
///         const $$_await = await (PROMISE_EXPR);
///         { const v = $$_await; ...body... }
///     };
/// so `v`'s type flows from the promise's resolved value (upstream
/// svelte2tsx behavior — see htmlxtojsx_v2/nodes/AwaitPendingCatchBlock.ts).
/// The outer `async () => {}` is a bare expression-statement — never
/// called, just provides the async context TS's `await` requires.
/// Necessary because `{#await}` can nest inside a sync snippet
/// callback (`children: () => {…}`), where bare `await` fires TS1375.
/// For `{:then {a, b, c}}` destructure patterns the binding text is
/// emitted verbatim as the destructure target.
#[allow(clippy::too_many_arguments)]
pub(crate) fn emit_await_then_branch(
    buf: &mut EmitBuffer,
    source: &str,
    promise_range: svn_core::Range,
    context_range: Option<&svn_core::Range>,
    body: &Fragment,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let promise_text = source
        .get(promise_range.start as usize..promise_range.end as usize)
        .unwrap_or("")
        .trim();
    if promise_text.is_empty() {
        emit_branch_with_binding(
            buf,
            source,
            context_range,
            body,
            depth,
            insts,
            action_counter,
        );
        return;
    }
    let indent = "    ".repeat(depth);
    let inner = "    ".repeat(depth + 1);
    let binding_text = context_range
        .and_then(|r| source.get(r.start as usize..r.end as usize))
        .map(str::trim)
        .filter(|s| !s.is_empty());

    let _ = writeln!(buf, "{indent};(async () => {{");
    match binding_text {
        Some(bind) => {
            let idents = all_identifiers(bind);
            let _ = writeln!(
                buf,
                "{inner}const $$_await = await ({promise_text}); const {bind} = $$_await;"
            );
            emit_template_body(buf, source, body, depth + 1, insts, action_counter);
            for ident in &idents {
                let _ = writeln!(buf, "{inner}void {ident};");
            }
        }
        None => {
            let _ = writeln!(
                buf,
                "{inner}const $$_await = await ({promise_text}); void $$_await;"
            );
            emit_template_body(buf, source, body, depth + 1, insts, action_counter);
        }
    }
    let _ = writeln!(buf, "{indent}}});");
}

/// Emit a lexical-scope block wrapping a `{#snippet name(params)}` body
/// so the snippet's parameter identifiers are in scope for references
/// inside the body (including component-prop checks on `<Component>`
/// nodes nested below the snippet).
///
/// Parameters are typed as `any` — we don't resolve their types from
/// the declared `Snippet<[...]>` shape here. For excess-prop and
/// "cannot find name" detection that's sufficient; precise param
/// typing would require threading the consumer's `Snippet<...>` into
/// the snippet body's type context, which v0.1 punts on.
///
/// Handles both identifier params (`foo, bar`) and destructure params
/// (`{months, weekdays}`, `[a, b]`) via `all_identifiers`. Default
/// values (`foo = 1`) have the default expression stripped before
/// identifier extraction.
pub(crate) fn emit_snippet_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &SnippetBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let params_text = source
        .get(b.parameters_range.start as usize..b.parameters_range.end as usize)
        .unwrap_or("")
        .trim();
    // `{#snippet name()}` with an empty parameter list — skip the
    // scope + placeholder binding entirely. `all_identifiers` never
    // returns an empty vec (it falls back to `__svn_each_unused`),
    // so we intercept the empty-params case before calling it.
    if params_text.is_empty() {
        emit_template_body(buf, source, &b.body, depth, insts, action_counter);
        return;
    }
    // Open a fresh block, then reference the ORIGINAL params text
    // inside an arrow function's signature — that keeps the user's
    // type annotations (e.g. `MouseEventHandler<HTMLButtonElement>`)
    // bound as references. Without this, types only used in a
    // snippet parameter list (and never in the script body) fire
    // TS6133 "declared but never read" on their import.
    //
    // We don't try to thread contextual typing through — top-level
    // snippets have no parent satisfies target to flow from. Each
    // user-named parameter gets re-declared inside the body as `const
    // <name>: any` so the snippet body type-checks without depending
    // on the arrow's inferred param types (which the `null as any`
    // cast erases anyway).
    let annotated = annotate_snippet_params(params_text);
    let idents = all_identifiers(params_text);
    // Emit the body INSIDE a no-op arrow function expression whose
    // params carry the user's type annotations. This serves two
    // purposes simultaneously:
    //   1. The arrow params are the binding introductions — their
    //      type annotations flow into the body, so a snippet like
    //      `{#snippet state(state: VideoState)}` gets `state: VideoState`
    //      bindings rather than `state: any`.
    //   2. Any type imported solely for a snippet param annotation
    //      (e.g. `MouseEventHandler<HTMLButtonElement>`) shows up as
    //      a reference in the emitted code, suppressing TS6133.
    //
    // We use a function *expression* (not a type) so default values
    // and optional-after-required orderings remain legal.
    let _ = writeln!(buf, "{indent}{{");
    let _ = writeln!(buf, "{indent}    void (({annotated}) => {{");
    emit_template_body(buf, source, &b.body, depth + 2, insts, action_counter);
    for ident in &idents {
        let _ = writeln!(buf, "{indent}        void {ident};");
    }
    let _ = writeln!(buf, "{indent}    }});");
    let _ = writeln!(buf, "{indent}}}");
}
