//! Rules that fire on `<svelte:*>` special elements.

use svn_parser::ast::{AttrValuePart, Attribute, SvelteElement, SvelteElementKind};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::element_rules::{AttrParent, visit_attribute};

pub fn visit(se: &SvelteElement, ctx: &mut LintContext<'_>, ancestors: &[String]) {
    // svelte_component_deprecated: `<svelte:component>` in runes mode.
    if ctx.runes && se.kind == SvelteElementKind::Component {
        let msg = messages::svelte_component_deprecated();
        ctx.emit(Code::svelte_component_deprecated, msg, se.range);
    }

    // svelte_self_deprecated: `<svelte:self>` in runes mode. Upstream
    // threads the component name + basename inferred from filename —
    // we don't have that at this layer yet; Phase A's best effort
    // uses fallback values.
    if ctx.runes && se.kind == SvelteElementKind::SelfRef {
        let msg = messages::svelte_self_deprecated("Self", "Self.svelte");
        ctx.emit(Code::svelte_self_deprecated, msg, se.range);
    }

    // Attribute checks: which AttrParent shape to route through.
    let parent = match se.kind {
        SvelteElementKind::Component | SvelteElementKind::SelfRef => {
            AttrParent::SvelteComponentLike
        }
        SvelteElementKind::Element => AttrParent::SvelteElement,
        _ => AttrParent::OtherSvelte,
    };
    for attr in &se.attributes {
        visit_attribute(attr, ctx, parent);
    }

    // Only route `<svelte:element>` through the a11y check — the
    // other svelte:* kinds (component/self/window/document/body/
    // head/options/fragment/boundary) aren't rendered elements.
    if se.kind == SvelteElementKind::Element {
        // svelte_element_invalid_this: `<svelte:element this="div">` (or
        // `this="h{n}"`) — `this` should be a single `{expression}`, not a
        // string / text-with-interpolation. Mirrors upstream's
        // `!is_expression_attribute(this)` warning (1-parse/state/element.js).
        if let Some(this_attr) = se.attributes.iter().find(|a| match a {
            Attribute::Plain(p) => p.name == "this",
            Attribute::Expression(e) => e.name == "this",
            Attribute::Shorthand(s) => s.name == "this",
            _ => false,
        }) {
            let is_expression = match this_attr {
                Attribute::Expression(_) | Attribute::Shorthand(_) => true,
                Attribute::Plain(p) => matches!(
                    p.value.as_ref(),
                    Some(v) if v.parts.len() == 1
                        && matches!(v.parts[0], AttrValuePart::Expression { .. })
                ),
                _ => false,
            };
            if !is_expression {
                let r = match this_attr {
                    Attribute::Plain(p) => p.range,
                    Attribute::Expression(e) => e.range,
                    Attribute::Shorthand(s) => s.range,
                    _ => se.range,
                };
                ctx.emit(
                    Code::svelte_element_invalid_this,
                    messages::svelte_element_invalid_this(),
                    r,
                );
            }
        }
        crate::rules::a11y_rules::visit_dynamic(se, ctx, ancestors);
    }
}
