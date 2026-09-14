//! `{#await}` / `{:then}` / `{:catch}` analyze pass — mirrors
//! upstream `htmlxtojsx_v2/nodes/AwaitPendingCatchBlock.ts`.

use crate::nodes::destructure::{destructured_value, resolve_template_expression};
use crate::template_scope::BoundIdent;
use crate::walker::{AnalyzeVisitor, ResolvedSlotExpr};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, b: &svn_parser::AwaitBlock) {
    v.pending_await_promise_range = Some(b.expression_range);
}

/// `{:then PAT}` branch — push each binding resolved from the awaited
/// promise. Reads `pending_await_promise_range` set by `visit_await_block`.
pub(crate) fn enter_then(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent]) {
    // Upstream: `__sveltets_2_unwrapPromiseLike(<promise>)`, the promise
    // expression resolved against the enclosing template scope, and a
    // destructured leaf as `((PATTERN) => leaf)(<that>)` (slot.ts).
    let awaited = v
        .pending_await_promise_range
        .and_then(|r| v.source.get(r.start as usize..r.end as usize))
        .and_then(|promise| resolve_template_expression(promise, &v.shadow))
        .map(|promise| format!("__svn_unwrap_promise_like({promise})"));
    for b in bindings {
        let resolved = awaited
            .as_deref()
            .map(|value| destructured_value(v.source, b, value));
        v.shadow.entries.push((b.name.clone(), resolved));
    }
}

/// `{:catch e}` branch — error type is `any` (matches upstream
/// `slot.ts:93`'s `__sveltets_2_any({})` resolution for CatchBlock
/// owners). Round-8 follow-up #3: destructure leaves resolve to `any`
/// too — upstream walks each leaf through resolveDestructuringAssignment
/// which returns `((${pattern}) => ${id})(any)` and TS narrows
/// `any[…]` to `any`, so the per-leaf type is `any` regardless of
/// pattern shape.
pub(crate) fn enter_catch(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent]) {
    for b in bindings {
        v.shadow.entries.push((
            b.name.clone(),
            Some(ResolvedSlotExpr::Type("any".to_string())),
        ));
    }
}
