//! `<element>` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Element.ts`.

use svn_parser::Element;

use crate::nodes::attribute::{WalkCtx, walk_attributes};
use crate::nodes::event_handler::collect_bubbled_dom_events;
use crate::nodes::let_directive::collect_slot_def;
use crate::walker::{AnalyzeVisitor, BubbledDomEventScope};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, e: &Element) {
    let ctx = WalkCtx { source: v.source };
    walk_attributes(
        &e.attributes,
        &mut v.summary,
        &mut v.counters,
        &ctx,
        Some(e.name.as_str()),
    );
    collect_bubbled_dom_events(&e.attributes, BubbledDomEventScope::Element, &mut v.summary);
    // `<slot [name="X"] [attr=…]>`: capture for emit's `slots:`
    // literal. Walks the attrs and skips any whose expression
    // references a name in the active shadow stack.
    if e.name.as_str() == "slot" {
        collect_slot_def(&e.attributes, v.source, &v.shadow, &mut v.summary);
    }
}
