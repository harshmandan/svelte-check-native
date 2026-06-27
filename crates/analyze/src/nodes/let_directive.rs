//! Slot-let analyze pass — two upstream sources:
//! `collect_slot_def` mirrors `svelte2tsx/nodes/slot.ts::handleSlot`
//! (the `<slot>` definition site → `$$slot_def`); `enter` mirrors
//! `htmlxtojsx_v2/nodes/Let.ts` + `slot.ts::getSingleSlotDef`
//! (the `<Comp let:foo>` consumer site).

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::Attribute;

use crate::nodes::destructure::{
    apply_default_narrow, default_typeof_expr, leading_identifier, project_destructure_path,
};
use crate::template_scope::BoundIdent;
use crate::walker::{
    AnalyzeVisitor, ResolvedSlotExpr, ResolverStack, SlotAttr, SlotAttrExpr, SlotDef,
    TemplateSummary,
};

/// Resolve a single `{expression}` slot-attr value into a `SlotAttr`
/// pushed onto `entries`. Shared by the `A::Expression` arm
/// (`<slot foo={bar}>`) and the single-`{expr}` `A::Plain` value case.
/// Mirrors the consumer-side expression resolution: shadow lookup +
/// the slot-attr rewriter, verbatim-splice when no shadowed names are
/// present, drop when a shadowed name can't be resolved.
fn collect_expression_attr(
    name: &SmolStr,
    expression_range: Range,
    source: &str,
    shadow: &ResolverStack,
    entries: &mut Vec<SlotAttr>,
) {
    let start = expression_range.start as usize;
    let end = expression_range.end as usize;
    let Some(text) = source.get(start..end) else {
        return;
    };
    let trimmed = text.trim();
    let lookup = |name: &str| shadow.lookup_resolved(name);
    // Path 1 — leading-ident shadowed. Try the TYPE-level rewriter
    // first (bare ident, member chain, computed member); it produces
    // the cleanest emit shape. If the expression is in a richer shape
    // the type-level rewriter doesn't handle (calls, ternaries, object
    // literals, etc.), fall through to the value-level walker so the
    // inner shadowed identifiers still get their typed casts. Bail if
    // neither produces a result.
    if let Some(head) = leading_identifier(trimmed)
        && shadow.lookup(head).is_some()
    {
        if let Some(rewritten) =
            crate::slot_attr_rewrite::rewrite_slot_attr_expr(trimmed, &lookup)
        {
            entries.push(SlotAttr::Prop {
                name: name.clone(),
                expr: SlotAttrExpr::Resolved(ResolvedSlotExpr::Type(rewritten)),
            });
            return;
        }
        if let crate::slot_attr_rewrite::ValueRewrite::Rewritten(rewritten) =
            crate::slot_attr_rewrite::rewrite_slot_attr_expr_value(trimmed, &lookup)
        {
            entries.push(SlotAttr::Prop {
                name: name.clone(),
                expr: SlotAttrExpr::Resolved(ResolvedSlotExpr::Value(rewritten)),
            });
        }
        return;
    }
    // Path 2 — leading-ident NOT shadowed (or no leading ident). Still
    // walk the whole expression for INNER shadowed identifiers (e.g.
    // `foo(item)`, `{ item }`, `fallback ?? item`,
    // `items.map(item => item.x)`). Without this the shadowed
    // identifier would leak verbatim to module scope; the value-level
    // walker rewrites each inner shadowed leaf to its typed cast and
    // splices the rest of the expression unchanged.
    match crate::slot_attr_rewrite::rewrite_slot_attr_expr_value(trimmed, &lookup) {
        crate::slot_attr_rewrite::ValueRewrite::Rewritten(rewritten) => {
            entries.push(SlotAttr::Prop {
                name: name.clone(),
                expr: SlotAttrExpr::Resolved(ResolvedSlotExpr::Value(rewritten)),
            });
        }
        crate::slot_attr_rewrite::ValueRewrite::NoEdits => {
            // No shadowed identifiers anywhere — splice source verbatim.
            entries.push(SlotAttr::Prop {
                name: name.clone(),
                expr: SlotAttrExpr::Range(expression_range),
            });
        }
        crate::slot_attr_rewrite::ValueRewrite::Bailed => {
            // Shadowed-but-unresolvable name somewhere in the
            // expression — drop the attr (would emit module-scope
            // identifiers that resolve to the wrong declaration).
        }
    }
}

/// Capture a `<slot [name="X"] [attr=…]>` site into
/// `summary.slot_defs`. Skips attrs whose expression references a
/// name in the active shadow stack — those need full scope
/// resolution to emit at module scope correctly. The slot is still
/// recorded (with a possibly-empty attrs list) so consumer-side
/// `<Comp let:foo>` destructure has SOMETHING to read from
/// `inst.$$slot_def[name]`.
pub(crate) fn collect_slot_def(
    attrs: &[Attribute],
    source: &str,
    shadow: &ResolverStack,
    summary: &mut TemplateSummary,
) {
    use svn_parser::{AttrValuePart, Attribute as A};
    let mut slot_name = SmolStr::new("default");
    let mut entries: Vec<SlotAttr> = Vec::new();
    for attr in attrs {
        match attr {
            A::Plain(p) if p.name.as_str() == "name" => {
                if let Some(v) = &p.value
                    && v.parts.len() == 1
                    && let AttrValuePart::Text { range } = &v.parts[0]
                {
                    slot_name = SmolStr::from(range.slice(source));
                }
            }
            A::Plain(p) => {
                // Plain literal attrs on `<slot>` other than `name=`
                // (e.g. `<slot kind="header">`). Single-text-part
                // values flow through as TS string literals so
                // consumer-side `<Comp let:kind>` destructure resolves
                // `kind` to `"header"`. Round-12 follow-up #5: multi-
                // part interpolated values (`<slot foo="a {b} c">`)
                // resolve to plain `string` (matches upstream
                // `slot.ts:46` which casts any multi-part attr to a
                // dummy string expression). Value-less boolean
                // shorthand is still skipped.
                if let Some(v) = &p.value
                    && v.parts.len() == 1
                    && let AttrValuePart::Text { range } = &v.parts[0]
                {
                    entries.push(SlotAttr::Prop {
                        name: p.name.clone(),
                        expr: SlotAttrExpr::Literal(range.slice(source).to_string()),
                    });
                } else if let Some(v) = &p.value
                    && v.parts.len() == 1
                    && let AttrValuePart::Expression {
                        expression_range, ..
                    } = &v.parts[0]
                {
                    // Single `{expr}` value (`<slot foo={bar}>`):
                    // resolve the expression the same way component /
                    // `let:` expression attrs do — shadow lookup + the
                    // slot-attr rewriter, verbatim-splice when no
                    // shadowed names are present. Multi-part
                    // interpolations still fall to the `string` cast
                    // below.
                    collect_expression_attr(
                        &p.name,
                        *expression_range,
                        source,
                        shadow,
                        &mut entries,
                    );
                } else if let Some(v) = &p.value
                    && !v.parts.is_empty()
                {
                    entries.push(SlotAttr::Prop {
                        name: p.name.clone(),
                        expr: SlotAttrExpr::Resolved(ResolvedSlotExpr::Type("string".to_string())),
                    });
                }
            }
            A::Expression(e) => {
                collect_expression_attr(&e.name, e.expression_range, source, shadow, &mut entries);
            }
            A::Shorthand(s) => {
                if let Some(resolved) = shadow.lookup(s.name.as_str()) {
                    if let Some(expr) = resolved {
                        entries.push(SlotAttr::Prop {
                            name: s.name.clone(),
                            expr: SlotAttrExpr::Resolved(expr.clone()),
                        });
                    }
                    // None or Some-but-non-bare-already-handled-above:
                    // shorthand always passes the bare-name check, so
                    // the only fall-through here is None (drop).
                    continue;
                }
                entries.push(SlotAttr::Prop {
                    name: s.name.clone(),
                    expr: SlotAttrExpr::Shorthand(s.name.clone()),
                });
            }
            A::Spread(spread) => {
                // SlotHandler PLAN Stage 3: `<slot {...rest}>` —
                // spreads survive as object-spread entries. The
                // expression must resolve through the same OXC
                // rewriter (when `rest` is shadowed) or splice
                // verbatim (module-scope identifier).
                let start = spread.expression_range.start as usize;
                let end = spread.expression_range.end as usize;
                let Some(text) = source.get(start..end) else {
                    continue;
                };
                let trimmed = text.trim();
                if let Some(head) = leading_identifier(trimmed)
                    && shadow.lookup(head).is_some()
                {
                    let lookup = |name: &str| shadow.lookup_resolved(name);
                    if let Some(rewritten) =
                        crate::slot_attr_rewrite::rewrite_slot_attr_expr(trimmed, &lookup)
                    {
                        entries.push(SlotAttr::Spread {
                            expr: SlotAttrExpr::Resolved(ResolvedSlotExpr::Type(rewritten)),
                        });
                    }
                    continue;
                }
                entries.push(SlotAttr::Spread {
                    expr: SlotAttrExpr::Range(spread.expression_range),
                });
            }
            // Directives and shapes we don't understand fall through
            // — drop them rather than emit something that resolves to
            // the wrong thing.
            _ => {}
        }
    }
    // Round-7 follow-up #4: upstream stores slots in a Map and
    // `set(slotName, attrs)` per `<slot name="x">` — multiple sites
    // for the same name resolve as later-wins. Native pre-fix pushed
    // every SlotDef in walk order and emit serialised them as
    // duplicate object keys (`{ 'x': {...}, 'x': {...} }`), which TS
    // accepts but flags as a noisy duplicate-key error and which
    // consumers would only ever see the last entry of regardless.
    // Replace any existing entry for `slot_name` with the new one so
    // emit produces a single key per name with the LAST occurrence's
    // attrs.
    let new_def = SlotDef {
        slot_name,
        attrs: entries,
    };
    if let Some(existing) = summary
        .slot_defs
        .iter_mut()
        .find(|d| d.slot_name == new_def.slot_name)
    {
        *existing = new_def;
    } else {
        summary.slot_defs.push(new_def);
    }
}

/// `<Comp let:foo>` / `<el let:foo>` scope — SlotHandler PLAN Stage 4.
/// Resolves each binding to
/// `__SvnComponentSlots<typeof Comp>['default']['foo']` (with
/// destructure-path projection). Reads `pending_let_owner` stashed by
/// `visit_component` / `visit_svelte_element`; falls through as
/// unresolvable when None (consumer-wrapper with `slot=`, dynamic
/// `<svelte:component this={EXPR}>` whose root isn't typeable,
/// plain DOM element with `let:`) so the slot-attr collector drops
/// references rather than splicing module scope.
///
/// Round-7 follow-up #2: each binding carries its own `slot_key_path`
/// (set by `collect_let_directive_bindings`). For shorthand and
/// bare-ident-alias forms the path is the directive name (e.g.
/// `["foo"]` for both `let:foo` and `let:foo={bar}`). Pre-fix native
/// used the BoundIdent's `name` as the slot key, so an alias
/// `let:foo={bar}` resolved `bar` to `…['default']['bar']` instead
/// of `…['default']['foo']`. Bindings without a path (today: any
/// destructure leaf) drop to None.
pub(crate) fn enter(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent]) {
    let owner = v.pending_let_owner.take();
    for b in bindings {
        let resolved = owner.as_ref().and_then(|info| {
            let path = b.slot_key_path.as_ref()?;
            if path.is_empty() {
                return None;
            }
            let root_expr = format!(
                "__SvnComponentSlots<typeof {root}>[{slot:?}]",
                root = info.component_root.as_str(),
                slot = info.slot_name.as_str(),
            );
            let projected = project_destructure_path(&root_expr, path);
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
        });
        v.shadow.entries.push((b.name.clone(), resolved));
    }
}
