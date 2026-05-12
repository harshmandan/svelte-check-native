//! `{#each}` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/EachBlock.ts`.

use crate::nodes::destructure::{
    apply_default_narrow, default_typeof_expr, items_typeof_expr, project_destructure_path,
};
use crate::template_scope::BoundIdent;
use crate::walker::{AnalyzeVisitor, ResolvedSlotExpr};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, b: &svn_parser::EachBlock) {
    v.summary.each_block_count += 1;
    v.pending_each_items_range = Some(b.expression_range);
}

/// `{#each X as PAT [, INDEX]}` body — push each binding onto the
/// resolver stack with a type that projects through `__SvnEachItem`
/// down the destructure path. The matching `visit_each_block` stashed
/// the items expression range in `pending_each_items_range`;
/// consume it here.
pub(crate) fn enter(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent], has_index: bool) {
    // Convention from `template_scope`: when `has_index`
    // is true, the index identifier is the LAST binding;
    // every preceding entry is a context (item) binding.
    let items_range = v.pending_each_items_range.take();
    let context_count = if has_index {
        bindings.len().saturating_sub(1)
    } else {
        bindings.len()
    };
    let items_text = items_range.and_then(|r| {
        v.source
            .get(r.start as usize..r.end as usize)
            .map(|s| s.trim().to_string())
    });
    // Round-7 follow-up #3: destructured patterns now
    // carry a `destructure_path` per leaf binding, so
    // `{#each rows as { id }}` resolves `id` to the
    // element's `['id']` slice. Mirrors upstream's
    // `((${destructuring}) => ${id})(__sveltets_2_unwrapArr(items))`
    // IIFE shape, but at TYPE level.
    for (i, b) in bindings.iter().enumerate() {
        let resolved = if has_index && i == context_count {
            // Index — always `number`.
            Some(ResolvedSlotExpr::Type("number".to_string()))
        } else if let Some(items) = items_text.as_deref() {
            // Round-12 follow-up #1: `typeof <items>` is
            // only legal when `<items>` is a bare
            // identifier or dotted member chain. For
            // expressions that aren't directly typeof-able
            // (calls, indexing, ternaries, etc.) build
            // the items type via a typeof-safe stand-in:
            //   - call on typeof-safe callee →
            //     `ReturnType<typeof <callee>>`
            //   - anything else → `any` (element type
            //     becomes `any`, which is permissive but
            //     parses cleanly; pre-fix produced a
            //     parse-error like `typeof getRows()`).
            // Round-14 #4: route through the
            // `__SvnEachItem<T>` shim instead of an
            // inline `T extends Iterable<infer U> ? U
            // : never` projection. The shim has the
            // `0 extends 1 & T ? any` guard for
            // `any`-preservation and an `ArrayLike`
            // branch (so plain `{ length: N }` shapes
            // resolve too), matching upstream's
            // `__sveltets_2_each` distribution.
            let items_ty = items_typeof_expr(items);
            let element_ty = format!("__SvnEachItem<{items_ty}>");
            // Round-15 #4: when the binding sits under an
            // AssignmentPattern, switch to upstream's
            // `((PATTERN) => name)(value)` IIFE shape
            // (`slot.ts:117`). TypeScript evaluates the
            // destructure with the actual default
            // expression, so object / array / template-
            // literal defaults preserve precise types
            // instead of collapsing to `Exclude<…,
            // undefined>` (or worse, leaking interpolated
            // template syntax into a TS type position).
            if b.has_default
                && let Some(pat_range) = b.pattern_source_range
                && let Some(pat_source) =
                    v.source.get(pat_range.start as usize..pat_range.end as usize)
            {
                Some(ResolvedSlotExpr::Value(format!(
                    "(({pat}) => {leaf})(undefined as any as ({element_ty}))",
                    pat = pat_source.trim(),
                    leaf = b.name.as_str(),
                )))
            } else {
                let projected = match b.destructure_path.as_deref() {
                    Some(path) => project_destructure_path(&element_ty, path),
                    None => element_ty,
                };
                let default_t = b.default_value_range.and_then(|r| {
                    v.source
                        .get(r.start as usize..r.end as usize)
                        .and_then(default_typeof_expr)
                });
                Some(ResolvedSlotExpr::Type(apply_default_narrow(
                    projected,
                    b.has_default,
                    default_t,
                )))
            }
        } else {
            None
        };
        v.shadow.entries.push((b.name.clone(), resolved));
    }
}
