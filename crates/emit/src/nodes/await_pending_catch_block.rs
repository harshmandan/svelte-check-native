//! `{#await}` then-branch and the `{:then}` / `{:catch}` branch helper.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/AwaitPendingCatchBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::Fragment;

use crate::emit_buffer::EmitBuffer;
use crate::{all_identifiers, emit_is_ts, emit_template_body};

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
///
/// ```text
///     ;async () => {
///         const $$_await = await (PROMISE_EXPR);
///         { const v = $$_await; ...body... }
///     };
/// ```
///
/// so `v`'s type flows from the promise's resolved value (matches
/// upstream svelte2tsx's `AwaitPendingCatchBlock.ts`). The outer
/// `async () => {}` is a bare expression-statement — never called,
/// just provides the async context TS's `await` requires. Necessary
/// because `{#await}` can nest inside a sync snippet callback
/// (`children: () => {…}`), where bare `await` fires TS1375.
///
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
