//! `{#snippet}` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/SnippetBlock.ts`. Also handles the `Fragment`
//! scope-bracket case: both push bindings as unresolvable (`None`)
//! since they share the "no upstream-equivalent slot resolution"
//! shape (per SlotHandler PLAN §6 "things not to do" for snippets,
//! plus Fragment scopes don't declare any bindings anyway).

use crate::template_scope::BoundIdent;
use crate::walker::AnalyzeVisitor;

/// Push each binding as unresolved (`None`). Used by both the
/// Snippet and Fragment scope arms of `enter_scope`.
pub(crate) fn enter_unresolved(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent]) {
    for b in bindings {
        v.shadow.entries.push((b.name.clone(), None));
    }
}
