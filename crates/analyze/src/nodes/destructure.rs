//! Destructure / typeof / narrowing helpers — pure free functions
//! shared by the per-node passes. No direct upstream equivalent;
//! upstream inlines these inside each node-handler file.

use smol_str::SmolStr;
use svn_core::Range;

/// Return the leading identifier of an expression source slice — the
/// run of identifier-valid bytes from the start, before any `.`,
/// `[`, `?.`, `(`, whitespace, or operator. For `item.id` returns
/// `"item"`; for `rest[0]` returns `"rest"`; for `user?.name` returns
/// `"user"`. Returns None when the slice doesn't start with an
/// identifier (e.g. `1 + foo`, `(x).y`).
///
/// Used by `collect_slot_def` to suppress slot-attr expressions whose
/// root binding is shadowed by an active template-scope let/each
/// binding — bare-identifier check alone misses member-access /
/// optional-chain / index-access shapes.
pub(crate) fn leading_identifier(s: &str) -> Option<&str> {
    let mut chars = s.char_indices();
    let (_, first) = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    let mut end = s.len();
    for (i, c) in chars {
        if !is_ident_continue(c) {
            end = i;
            break;
        }
    }
    Some(&s[..end])
}

/// A template expression with every identifier that names an enclosing
/// template binding replaced by that binding's own resolution — upstream
/// `SlotHandler.resolveExpression`. `None` when a shadowed name has no
/// resolution (the caller then drops the binding rather than resolving
/// it to the wrong declaration).
pub(crate) fn resolve_template_expression(
    text: &str,
    shadow: &crate::walker::ResolverStack,
) -> Option<String> {
    use crate::slot_attr_rewrite::{ValueRewrite, rewrite_slot_attr_expr_value};
    match rewrite_slot_attr_expr_value(text, &|name| shadow.lookup_resolved(name)) {
        ValueRewrite::Rewritten(s) => Some(s),
        ValueRewrite::NoEdits => Some(text.trim().to_string()),
        ValueRewrite::Bailed => None,
    }
}

/// The value expression for one binding declared by a pattern over
/// `value`: the value itself for a plain identifier, upstream's
/// `((PATTERN) => leaf)(value)` for a destructured leaf (slot.ts
/// `resolveDestructuringAssignment`), so TS evaluates the destructure
/// — defaults included — exactly as written.
pub(crate) fn destructured_value(
    source: &str,
    b: &crate::template_scope::BoundIdent,
    value: &str,
) -> crate::walker::ResolvedSlotExpr {
    use crate::walker::ResolvedSlotExpr;
    let is_leaf = b.destructure_path.is_some() || b.has_default;
    if is_leaf
        && let Some(range) = b.pattern_source_range
        && let Some(pattern) = source.get(range.start as usize..range.end as usize)
    {
        return ResolvedSlotExpr::Value(format!(
            "(({pattern}) => {leaf})({value})",
            pattern = pattern.trim(),
            leaf = b.name.as_str(),
        ));
    }
    ResolvedSlotExpr::Value(value.to_string())
}

/// If the byte range covers a single ECMAScript identifier (with optional
/// surrounding whitespace), return it.
pub(crate) fn simple_identifier_in(source: &str, range: Range) -> Option<SmolStr> {
    let slice = source.get(range.start as usize..range.end as usize)?.trim();
    if slice.is_empty() {
        return None;
    }
    let mut chars = slice.chars();
    let first = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    if chars.all(is_ident_continue) {
        Some(SmolStr::from(slice))
    } else {
        None
    }
}

#[inline]
fn is_ident_start(c: char) -> bool {
    // `_` is already covered by XID_Start; `$` is a JS-only carve-out.
    unicode_ident::is_xid_start(c) || c == '_' || c == '$'
}

#[inline]
fn is_ident_continue(c: char) -> bool {
    // `$` is a JS-only carve-out (`_` is already a XID_Continue char).
    unicode_ident::is_xid_continue(c) || c == '$'
}
