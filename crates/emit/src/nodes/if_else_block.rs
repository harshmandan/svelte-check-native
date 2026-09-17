//! `{#if}` / `{:else if}` / `{:else}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/IfElseBlock.ts`.
//!
//! [`emit_if_block`] drives the whole `if`/`else if`/`else` chain itself
//! (rather than recursing through `emit_template_node`) because
//! `{:else if}` arms are a structurally flat list, not a nested tree.

use std::collections::HashMap;
use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;
use crate::emit_template_body;

/// Emit a `{#if cond}…{:else if c2}…{:else}…{/if}` block as a real
/// `if (cond) {} else if (c2) {} else {}` chain so tsgo's
/// control-flow analysis narrows union / nullable / type-guard
/// references inside each arm. Without this, `{#if shape.kind ===
/// 'circle'}` leaves `shape.radius` reading as TS2339 inside the
/// nested component-prop check, and `{#if maybe}{...maybe...}{/if}`
/// reads as "`maybe` is possibly undefined".
///
/// Conditions are wrapped in an extra pair of parens to stay robust
/// against operator-precedence oddities in the raw source text.
pub(crate) fn emit_if_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &svn_parser::IfBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let main_cond = source
        .get(b.condition_range.start as usize..b.condition_range.end as usize)
        .unwrap_or("true")
        .trim_ascii();
    // R-Conv #20 (B2 #4): splice the condition with append_with_source
    // so any diagnostic firing on the condition expression
    // (TS2367 "comparison appears to be unintentional", TS18047
    // "possibly null/undefined") reverse-maps to the user's
    // `{#if EXPR}` source span. Pre-fix `writeln!` wrote the text
    // raw — diagnostics without a TokenMap entry got dropped.
    let _ = write!(buf, "{indent}if ((");
    if main_cond.is_empty() {
        buf.push_str("true");
    } else {
        buf.append_with_source(main_cond, b.condition_range);
    }
    buf.push_str(")) {\n");
    emit_template_body(buf, source, &b.consequent, depth + 1, insts, action_counter);
    for arm in &b.elseif_arms {
        let arm_cond = source
            .get(arm.condition_range.start as usize..arm.condition_range.end as usize)
            .unwrap_or("true")
            .trim_ascii();
        let _ = write!(buf, "{indent}}} else if ((");
        if arm_cond.is_empty() {
            buf.push_str("true");
        } else {
            buf.append_with_source(arm_cond, arm.condition_range);
        }
        buf.push_str(")) {\n");
        emit_template_body(buf, source, &arm.body, depth + 1, insts, action_counter);
    }
    if let Some(alt) = &b.alternate {
        let _ = writeln!(buf, "{indent}}} else {{");
        emit_template_body(buf, source, alt, depth + 1, insts, action_counter);
    }
    let _ = writeln!(buf, "{indent}}}");
}
