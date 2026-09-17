//! `{#await}` blocks.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/AwaitPendingCatchBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;
use crate::emit_template_body;

/// Emit `{#await PROMISE}…{:then v}…{:catch e}…{/await}` the way
/// `handleAwait` does:
///
/// ```text
/// {
///     …pending…
///     try {
///         const $$_value = await (PROMISE);
///         { const v = $$_value; …then… }
///     } catch($$_e) { const e = __svn_any(); …catch… }
/// }
/// ```
///
/// The `try` appears only with a `{:catch}`, `$$_value` only with a
/// `then` binding. The `await` is inline — no wrapping closure — so
/// narrowing from enclosing blocks survives into the branches; the
/// async context comes from the render function and snippet bodies.
/// The `then` body is emitted when the block has no pending body, or
/// when the `{:then}` branch has children.
pub(crate) fn emit_await_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &svn_parser::AwaitBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let inner = "    ".repeat(depth + 1);
    let binding = |range: Option<&svn_core::Range>| {
        range.and_then(|r| {
            let raw = source.get(r.start as usize..r.end as usize)?;
            let text = raw.trim();
            (!text.is_empty()).then(|| {
                let start = r.start + (raw.len() - raw.trim_start().len()) as u32;
                (text, svn_core::Range::new(start, start + text.len() as u32))
            })
        })
    };
    let value = binding(
        b.then_branch
            .as_ref()
            .and_then(|t| t.context_range.as_ref()),
    );
    let error = binding(
        b.catch_branch
            .as_ref()
            .and_then(|c| c.context_range.as_ref()),
    );
    let has_catch = b.catch_branch.is_some();

    let _ = writeln!(buf, "{indent}{{");
    if let Some(p) = &b.pending {
        emit_template_body(buf, source, p, depth + 1, insts, action_counter);
    }
    buf.push_str(&inner);
    if has_catch {
        buf.push_str("try { ");
    }
    if value.is_some() {
        buf.push_str("const $$_value = ");
    }
    buf.push_str("await (");
    let promise_raw = source
        .get(b.expression_range.start as usize..b.expression_range.end as usize)
        .unwrap_or("");
    let promise = promise_raw.trim();
    let promise_start =
        b.expression_range.start + (promise_raw.len() - promise_raw.trim_start().len()) as u32;
    buf.append_with_source(
        promise,
        svn_core::Range::new(promise_start, promise_start + promise.len() as u32),
    );
    buf.push_str(");\n");
    if let Some((text, range)) = value {
        let _ = write!(buf, "{inner}{{ const ");
        buf.append_with_source(text, range);
        buf.push_str(" = $$_value;\n");
    }
    if let Some(t) = &b.then_branch
        && (b.pending.is_none() || !t.body.nodes.is_empty())
    {
        emit_template_body(buf, source, &t.body, depth + 1, insts, action_counter);
    }
    if value.is_some() {
        let _ = writeln!(buf, "{inner}}}");
    }
    if let Some(c) = &b.catch_branch {
        let _ = write!(buf, "{inner}}} catch ($$_e) {{");
        if let Some((text, range)) = error {
            buf.push_str(" const ");
            buf.append_with_source(text, range);
            buf.push_str(" = __svn_any();");
        }
        buf.push('\n');
        emit_template_body(buf, source, &c.body, depth + 1, insts, action_counter);
        let _ = writeln!(buf, "{inner}}}");
    }
    let _ = writeln!(buf, "{indent}}}");
}
