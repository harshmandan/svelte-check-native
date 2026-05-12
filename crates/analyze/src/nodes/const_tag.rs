//! `{@const}` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/ConstTag.ts`.

use smol_str::SmolStr;
use svn_core::Range;

use crate::nodes::destructure::is_destructure;
use crate::walker::AnalyzeVisitor;

pub(crate) fn visit_at_const(
    v: &mut AnalyzeVisitor<'_>,
    bound_names: &[SmolStr],
    _expr_range: Range,
) {
    // Push every bound name onto the shadow so subsequent
    // slot-attr / let-directive sites in the same fragment treat
    // them as scope-local. Destructure `{@const}` forms
    // (`{@const { a, b } = X}`) emit multiple names; bare
    // `{@const NAME = X}` emits one. The walker's fragment-level
    // bracket truncates them at exit.
    //
    // For the emit's `let NAME: any;` summary list, the legacy
    // shape is one name per `{@const}` (bare-identifier form
    // only). Destructure forms aren't currently surfaced in
    // `at_const_names` because emit doesn't yet declare per-
    // identifier `let` for them — that's tracked separately as
    // a follow-up. Until then, only push the FIRST name to the
    // summary list (matches pre-Phase-4 behaviour where
    // destructure forms were skipped entirely from the list).
    if let Some(first) = bound_names.first()
        && !is_destructure(bound_names)
        && v.counters.at_const_seen.insert(first.clone())
    {
        v.summary.at_const_names.push(first.clone());
    }
    for name in bound_names {
        // `{@const NAME = expr}` introduces a template-scope
        // binding without a value source we can rewrite (the
        // initialiser walks in the parent scope, but the bound
        // name itself is opaque to the slot resolver). Push as
        // `None` — bound but unresolvable. Slot-attr collection
        // drops references rather than splicing module-scope.
        v.shadow.entries.push((name.clone(), None));
    }
}
