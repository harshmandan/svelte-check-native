//! `{#await}` / `{:then}` / `{:catch}` analyze pass — mirrors
//! upstream `htmlxtojsx_v2/nodes/AwaitPendingCatchBlock.ts`.

use crate::nodes::destructure::{
    apply_default_narrow, default_typeof_expr, items_typeof_expr, project_destructure_path,
};
use crate::template_scope::BoundIdent;
use crate::walker::{AnalyzeVisitor, ResolvedSlotExpr};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, b: &svn_parser::AwaitBlock) {
    v.pending_await_promise_range = Some(b.expression_range);
}

/// `{:then PAT}` branch — push each binding with a type derived
/// from `Awaited<<promise>>` projected down the destructure path.
/// Reads `pending_await_promise_range` set by `visit_await_block`.
pub(crate) fn enter_then(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent]) {
    let promise_range = v.pending_await_promise_range;
    // Unwrap the promise type once for every binding. Non-typeof-able
    // promise expressions (calls, chains) fall back to a typeof-safe
    // stand-in inside `items_typeof_expr`, so the promise becomes
    // `Promise<any>` at worst, which `Awaited` unwraps to `any` — a bare
    // `Awaited<typeof load()>` would otherwise be a parse error.
    let unwrapped = promise_range
        .and_then(|r| v.source.get(r.start as usize..r.end as usize))
        .map(|p| {
            let promise_ty = items_typeof_expr(p); // trims internally
            format!("(Awaited<{promise_ty}>)")
        });
    // Destructured `{:then { x }}` resolves each leaf via the binding's
    // `destructure_path`. Bare `{:then v}` keeps the unwrapped promise
    // type directly (no path suffix).
    for b in bindings {
        let resolved = unwrapped.as_deref().map(|unwrapped| {
            // Switch to the IIFE shape when the binding has a default,
            // matching the each-block branch.
            if b.has_default
                && let Some(pat_range) = b.pattern_source_range
                && let Some(pat_source) =
                    v.source.get(pat_range.start as usize..pat_range.end as usize)
            {
                return ResolvedSlotExpr::Value(format!(
                    "(({pat}) => {leaf})(undefined as any as ({unwrapped}))",
                    pat = pat_source.trim(),
                    leaf = b.name.as_str(),
                ));
            }
            let projected = match b.destructure_path.as_deref() {
                Some(path) => project_destructure_path(unwrapped, path),
                None => unwrapped.to_string(),
            };
            let default_t = b.default_value_range.and_then(|r| {
                v.source
                    .get(r.start as usize..r.end as usize)
                    .and_then(default_typeof_expr)
            });
            ResolvedSlotExpr::Type(apply_default_narrow(projected, b.has_default, default_t))
        });
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
