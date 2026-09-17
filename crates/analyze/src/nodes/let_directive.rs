//! Slot-let analyze pass — two upstream sources:
//! `collect_slot_def` mirrors `svelte2tsx/nodes/slot.ts::handleSlot`
//! (the `<slot>` definition site → `$$slot_def`); `enter` mirrors
//! `htmlxtojsx_v2/nodes/Let.ts` + `slot.ts::getSingleSlotDef`
//! (the `<Comp let:foo>` consumer site).

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::Attribute;

use crate::nodes::destructure::leading_identifier;
use crate::template_scope::{BoundIdent, DestructureSeg};
use crate::walker::{
    AnalyzeVisitor, LetOwnerInfo, ResolvedSlotExpr, ResolverStack, SlotAttr, SlotAttrExpr, SlotDef,
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
        if let Some(rewritten) = crate::slot_attr_rewrite::rewrite_slot_attr_expr(trimmed, &lookup)
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

/// The name an attribute is written with (a directive's name follows
/// its `prefix:`); `None` for spreads and comments.
fn attribute_name(attr: &Attribute) -> Option<&str> {
    match attr {
        Attribute::Plain(p) => Some(p.name.as_str()),
        Attribute::Expression(e) => Some(e.name.as_str()),
        Attribute::Shorthand(s) => Some(s.name.as_str()),
        Attribute::Directive(d) => Some(d.name.as_str()),
        Attribute::Spread(_) | Attribute::Comment(_) => None,
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
    // svelte2tsx's `handleSlot` names the slot after the raw text of the
    // first attribute called `name`'s first value chunk; a `{…}` value
    // has no raw text, which makes the key `undefined`.
    let slot_name = match attrs.iter().find(|a| attribute_name(a) == Some("name")) {
        None => SmolStr::new("default"),
        Some(A::Plain(p)) => match p.value.as_ref().map(|v| v.parts.first()) {
            Some(Some(AttrValuePart::Text { range })) => SmolStr::from(range.slice(source)),
            Some(None) => SmolStr::default(),
            _ => SmolStr::new("undefined"),
        },
        Some(_) => SmolStr::new("undefined"),
    };
    let mut entries: Vec<SlotAttr> = Vec::new();
    for attr in attrs {
        // Every attribute called `name` names the slot and is not a
        // slot prop.
        if attribute_name(attr) == Some("name") {
            continue;
        }
        match attr {
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
                    } else if let crate::slot_attr_rewrite::ValueRewrite::Rewritten(rewritten) =
                        crate::slot_attr_rewrite::rewrite_slot_attr_expr_value(trimmed, &lookup)
                    {
                        // A value-resolved root (an each / await binding):
                        // spread the value, as upstream does.
                        entries.push(SlotAttr::Spread {
                            expr: SlotAttrExpr::Resolved(ResolvedSlotExpr::Value(rewritten)),
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

/// `<Comp let:foo>` / `<el slot="x" let:foo>` scope. svelte2tsx's slot
/// resolver (`slot.ts` `resolveLet` / `resolveDestructuringAssignmentForLet`)
/// resolves each name at value level against the owning component's
/// instance: `__sveltets_2_instanceOf(Comp).$$slot_def['slot'].foo`,
/// and a destructured leaf as `((PATTERN) => leaf)(<that>)`, so TS
/// types the leaf from the real destructure. `<svelte:component>` and
/// `<svelte:self>` resolve through an undeclared helper, i.e. `any`.
/// With no owner (an element that fills no named slot of a component)
/// svelte2tsx does not track the names, and nothing is pushed.
pub(crate) fn enter(v: &mut AnalyzeVisitor<'_>, bindings: &[BoundIdent]) {
    let Some(owner) = v.pending_let_owner.take() else {
        return;
    };
    for b in bindings {
        let Some(path) = b.slot_key_path.as_ref() else {
            continue;
        };
        let Some(DestructureSeg::Key(let_name)) = path.first() else {
            continue;
        };
        // svelte2tsx turns only the outermost `{…}` / `[…]` of the
        // directive's expression into a pattern before collecting its
        // identifiers, so only a plain identifier directly inside it is
        // declared; nested patterns, defaults and rests keep their names
        // as written.
        let declared = match path.len() {
            1 => true,
            2 => {
                matches!(
                    path[1],
                    DestructureSeg::Key(_) | DestructureSeg::KeyTypeof(_)
                ) && !b.has_default
                    && !b.inside_rest
            }
            _ => false,
        };
        if !declared {
            continue;
        }
        let resolved = match &owner.component {
            None => ResolvedSlotExpr::Type("any".to_string()),
            Some(component) => {
                let slot = owner.slot_name.replace('\\', "\\\\").replace('\'', "\\'");
                let base =
                    format!("__svn_instance_of({component}).$$slot_def['{slot}'].{let_name}");
                match destructure_pattern(v.source, b) {
                    Some(pattern) => ResolvedSlotExpr::Value(format!(
                        "(({pattern}) => {leaf})({base})",
                        leaf = b.name.as_str(),
                    )),
                    None => ResolvedSlotExpr::Value(base),
                }
            }
        };
        v.shadow.entries.push((b.name.clone(), Some(resolved)));
    }
}

/// The destructure pattern a `let:NAME={PATTERN}` binding is a leaf of;
/// `None` for the shorthand and plain-alias forms.
fn destructure_pattern<'s>(source: &'s str, b: &BoundIdent) -> Option<&'s str> {
    let range = b.pattern_source_range?;
    let pattern = source.get(range.start as usize..range.end as usize)?.trim();
    let is_plain_identifier = !pattern.is_empty()
        && pattern
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '$');
    (!is_plain_identifier).then_some(pattern)
}

/// svelte2tsx's `getSlotName`: the raw text of the first value chunk of
/// the first attribute called `slot`, when that is non-empty text.
fn slot_name_of<'s>(attrs: &[Attribute], source: &'s str) -> Option<&'s str> {
    let first = attrs.iter().find(|a| attribute_name(a) == Some("slot"))?;
    let Attribute::Plain(p) = first else {
        return None;
    };
    match p.value.as_ref()?.parts.first()? {
        svn_parser::AttrValuePart::Text { range } => {
            Some(range.slice(source)).filter(|t| !t.is_empty())
        }
        svn_parser::AttrValuePart::Expression { .. } => None,
    }
}

pub(crate) fn has_let_directive(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|a| matches!(a, Attribute::Directive(d) if d.kind == svn_parser::DirectiveKind::Let))
}

/// Owner bookkeeping for a component-like node (`<Comp>`,
/// `<svelte:component>`, `<svelte:self>`) about to have its children
/// walked. svelte2tsx's `handleComponentLet` resolves the component's
/// own `let:` directives against its default slot, and those of each
/// direct child that names a slot against that slot — the latter first
/// (the resolution a child's own component would give is never used).
pub(crate) fn enter_component_like(
    v: &mut AnalyzeVisitor<'_>,
    start: u32,
    attrs: &[Attribute],
    children: &svn_parser::Fragment,
    component: Option<SmolStr>,
) {
    let registered = v.slot_let_owners.remove(&start);
    if has_let_directive(attrs) {
        v.pending_let_owner = Some(registered.unwrap_or_else(|| LetOwnerInfo {
            component: component.clone(),
            slot_name: SmolStr::new("default"),
        }));
    }
    for child in &children.nodes {
        let child_attrs = match child {
            svn_parser::Node::Element(e) => &e.attributes,
            svn_parser::Node::Component(c) => &c.attributes,
            svn_parser::Node::SvelteElement(e) => &e.attributes,
            _ => continue,
        };
        if !has_let_directive(child_attrs) {
            continue;
        }
        if let Some(slot) = slot_name_of(child_attrs, v.source) {
            v.slot_let_owners.insert(
                child.range().start,
                LetOwnerInfo {
                    component: component.clone(),
                    slot_name: SmolStr::new(slot),
                },
            );
        }
    }
}

/// Owner bookkeeping for a node that is not component-like: its `let:`
/// directives are resolved only when its parent component registered
/// it as filling a named slot.
pub(crate) fn enter_element_like(v: &mut AnalyzeVisitor<'_>, start: u32, attrs: &[Attribute]) {
    let registered = v.slot_let_owners.remove(&start);
    if has_let_directive(attrs) {
        v.pending_let_owner = registered;
    }
}
