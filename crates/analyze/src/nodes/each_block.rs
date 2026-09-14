//! `{#each}` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/EachBlock.ts`.

use crate::nodes::destructure::{destructured_value, resolve_template_expression};
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
    // Upstream resolves an each binding at value level:
    // `__sveltets_2_unwrapArr(<items>)`, with outer template bindings
    // inside `<items>` already replaced by their own resolutions, and a
    // destructured leaf as `((PATTERN) => leaf)(<that>)` (slot.ts). TS
    // then types the leaf from the real expression — overloads, type
    // predicates and `??` fallbacks included — instead of from a type
    // derived from the expression text.
    let items_value = items_range
        .and_then(|r| v.source.get(r.start as usize..r.end as usize))
        .and_then(|items| resolve_template_expression(items, &v.shadow))
        .map(|items| format!("__svn_unwrap_arr({items})"));
    for (i, b) in bindings.iter().enumerate() {
        let resolved = if has_index && i == context_count {
            // Index — always `number`.
            Some(ResolvedSlotExpr::Type("number".to_string()))
        } else {
            items_value
                .as_deref()
                .map(|value| destructured_value(v.source, b, value))
        };
        v.shadow.entries.push((b.name.clone(), resolved));
    }
}
