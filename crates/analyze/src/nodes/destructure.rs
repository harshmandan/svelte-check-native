//! Destructure / typeof / narrowing helpers — pure free functions
//! shared by the per-node passes. No direct upstream equivalent;
//! upstream inlines these inside each node-handler file.

use std::fmt::Write as _;

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

/// Returns true iff `s` is a bare identifier or a dotted chain of
/// identifiers (`a`, `a.b`, `a.b.c`, …). Whitespace and other
/// tokens are not allowed inside the chain.
pub(crate) fn is_typeof_safe_chain(s: &str) -> bool {
    if s.is_empty() {
        return false;
    }
    let mut at_segment_start = true;
    for ch in s.chars() {
        if at_segment_start {
            if !is_ident_start(ch) {
                return false;
            }
            at_segment_start = false;
        } else if ch == '.' {
            at_segment_start = true;
        } else if !is_ident_continue(ch) {
            return false;
        }
    }
    !at_segment_start
}

/// Round-12 follow-up #2: when a destructure leaf carries a default
/// (`{ a = 1 }`), wrap the projected type in `Exclude<…, undefined>`.
/// At the value level, the default kicks in only when the source's
/// slice is undefined; the destructured local then has the source's
/// type minus undefined PLUS the default's type. Round-13 #4 unions
/// the default's typeof-derived type when extractable from a
/// literal/identifier/dotted-chain/typeof-safe-call source — common
/// cases like `{ a = 1 }` (default type `1`) or `{:then v = 0}`
/// (default type `0`) so the binding type matches upstream's IIFE
/// narrowing even when the source itself can't supply the fallback.
pub(crate) fn apply_default_narrow(
    projected: String,
    has_default: bool,
    default_typeof: Option<String>,
) -> String {
    if !has_default {
        return projected;
    }
    let excluded = format!("Exclude<{projected}, undefined>");
    match default_typeof {
        Some(t) => format!("({excluded} | {t})"),
        None => excluded,
    }
}

/// Derive a TS type expression from a default-value source slice.
/// Recognised shapes:
///
/// - string literal (`'fallback'` / `"x"` / `` `tpl` ``) → the
///   literal type itself.
/// - boolean / null / undefined / numeric literal → the literal type.
/// - bare identifier / dotted chain                → `typeof X`.
/// - anything else                                 → `None` (caller
///   falls back to `Exclude<…, undefined>` only).
pub(crate) fn default_typeof_expr(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    // Literal-type passthroughs. These are valid TS types as-is
    // (`'fallback'` is the string-literal type, `1` is the numeric-
    // literal type, etc.).
    if trimmed.starts_with('"') || trimmed.starts_with('\'') {
        return Some(trimmed.to_string());
    }
    // A backtick template is a valid string-/template-literal TYPE only when it
    // has no `${…}` interpolation. Interpolated text (`` `${y}px` ``) would read
    // `y` as a TYPE reference (spurious TS2749); fall back to Exclude<…> instead.
    if trimmed.starts_with('`') {
        if trimmed.contains("${") {
            return None;
        }
        return Some(trimmed.to_string());
    }
    if trimmed == "true" || trimmed == "false" || trimmed == "null" || trimmed == "undefined" {
        return Some(trimmed.to_string());
    }
    if trimmed.parse::<f64>().is_ok() {
        return Some(trimmed.to_string());
    }
    // A bare identifier or dotted chain: `typeof X`. Anything else
    // would widen the projected leaf to `any`; skip the union instead.
    is_typeof_safe_chain(trimmed).then(|| format!("typeof {trimmed}"))
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

/// Round-7 follow-up #3 / Round-9 #4: project a `root` expression
/// down a destructure-segment chain. `Key` segments append bracket
/// access (`["name"]`); `ObjectRest` segments wrap the running
/// expression in `Omit<…, sibling1 | sibling2 | …>`. Numeric `Key`
/// segments stay quoted (TS treats `obj["0"]` and `obj[0]`
/// interchangeably for tuple/array access).
pub(crate) fn project_destructure_path(
    root: &str,
    path: &[crate::template_scope::DestructureSeg],
) -> String {
    let mut current = root.to_string();
    for seg in path {
        match seg {
            crate::template_scope::DestructureSeg::Key(k) => {
                current.push('[');
                let _ = write!(current, "{:?}", k.as_str());
                current.push(']');
            }
            crate::template_scope::DestructureSeg::ObjectRest { siblings } => {
                if siblings.is_empty() {
                    // No siblings to subtract — rest IS the parent.
                    // (Pathological case: `{ ...rest }` with no other
                    // properties. Type stays as parent.)
                    continue;
                }
                // Round-12 #6: each sibling renders as either a
                // string-literal type (static keys) or `typeof <ident>`
                // (bare-ident computed keys). The union goes into
                // `Omit<parent, …>`'s second arg.
                let union = siblings
                    .iter()
                    .map(|s| match s {
                        crate::template_scope::ObjectRestSibling::Static(name) => {
                            format!("{:?}", name.as_str())
                        }
                        crate::template_scope::ObjectRestSibling::Typeof(name) => {
                            format!("typeof {}", name.as_str())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(" | ");
                current = format!("Omit<{current}, {union}>");
            }
            crate::template_scope::DestructureSeg::KeyTypeof(name) => {
                // Round-11 follow-up #5: computed-key with bare
                // identifier (`{ [k]: v }`). Project as
                // `parent[typeof k]`. The `typeof` lookup at the
                // type level gives whatever string-literal type the
                // identifier carries, which TS uses to index into
                // `parent`.
                current.push('[');
                current.push_str("typeof ");
                current.push_str(name.as_str());
                current.push(']');
            }
            crate::template_scope::DestructureSeg::ArrayRest { skip } => {
                // Round-10 #4 / Round-11 #4: tuple-tail extraction
                // with a variable-array fallback. The tuple-pattern
                // conditional `T extends readonly [unknown, ...infer
                // R]` doesn't reliably match variable arrays —
                // `string[]` doesn't structurally satisfy a fixed-
                // length tuple prefix, so the conditional falls
                // through to `never`. Layer a second branch that
                // catches the array case as `(infer U)[]` and
                // projects back to `U[]`:
                //
                //   T extends readonly [unknown, …(skip), ...infer R]
                //     ? R
                //     : T extends readonly (infer U)[]
                //       ? U[]
                //       : never
                let prefix = if *skip == 0 {
                    String::new()
                } else {
                    let mut p = String::new();
                    for i in 0..*skip {
                        if i > 0 {
                            p.push_str(", ");
                        }
                        p.push_str("unknown");
                    }
                    p.push_str(", ");
                    p
                };
                current = format!(
                    "({current} extends readonly [{prefix}...infer __svn_R] \
                     ? __svn_R \
                     : {current} extends readonly (infer __svn_U)[] \
                     ? __svn_U[] \
                     : never)"
                );
            }
        }
    }
    current
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

/// Whether `s` is a valid bare JS identifier (no dots, no
/// brackets, no whitespace). Used to gate the SlotHandler
/// let-owner resolver — only simple-named components like
/// `<Wrapper let:foo>` get the `__SvnComponentSlots<typeof Wrapper>`
/// projection. Dotted forms (`<UI.Dropdown let:foo>`) would
/// produce malformed `typeof` references; those fall back to
/// the unresolved-shadow path.
pub(crate) fn is_simple_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_ident_start(first) && chars.all(is_ident_continue)
}
