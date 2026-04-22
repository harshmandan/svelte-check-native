//! Rules that fire on `<svelte:*>` special elements.

use svn_parser::ast::{SvelteElement, SvelteElementKind};

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
        crate::rules::a11y_rules::visit_dynamic(se, ctx, ancestors);
    }
}
