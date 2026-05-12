//! Attribute analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Attribute.ts`. Also hosts the directive
//! dispatcher (`walk_attributes` / `walk_directive`) since
//! directives reach analyze through the attribute list and the
//! dispatcher's only job is to route each directive to the
//! per-node handler.

use svn_parser::{Attribute, Directive, DirectiveKind};

use crate::nodes::action::handle_use_directive;
use crate::nodes::binding::handle_bind_directive;
use crate::walker::{Counters, TemplateSummary};

/// Per-walk context threaded through `walk_attributes` /
/// `walk_directive`. Currently a single-field source-text wrapper —
/// keeps the per-arm handlers' signatures uniform and leaves room
/// for additional contextual state without churning every handler.
pub(crate) struct WalkCtx<'src> {
    pub(crate) source: &'src str,
}

/// Return the literal string value of a plain attribute `name="LITERAL"`,
/// or None if the attribute is absent, quoted with an expression
/// interpolation, or bound via `name={expr}`. Used for context-aware
/// bind dispatch (`<input type="number" bind:value={...}>`).
pub(crate) fn walk_attributes(
    attrs: &[Attribute],
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
    parent_tag: Option<&str>,
) {
    for attr in attrs {
        if let Attribute::Directive(d) = attr {
            walk_directive(d, summary, counters, ctx, parent_tag);
        }
    }
}

/// Per-arm dispatcher. Only `Use` and `Bind` carry analyze logic
/// today; the remaining `DirectiveKind` variants (`On`, `Class`,
/// `Style`, `Transition`, `In`, `Out`, `Animate`, `Let`) fall
/// through to the `_ => {}` arm and are picked up elsewhere
/// (component-side `on:` collection in `inline_component`,
/// element-side `on:` bubbling in `event_handler`, etc.).
fn walk_directive(
    d: &Directive,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
    parent_tag: Option<&str>,
) {
    match d.kind {
        DirectiveKind::Use => handle_use_directive(d, summary, counters, parent_tag),
        DirectiveKind::Bind => handle_bind_directive(d, summary, counters, ctx.source),
        _ => {}
    }
}

pub fn literal_attr_value<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    for attr in attrs {
        let Attribute::Plain(p) = attr else {
            continue;
        };
        if p.name.as_str() != name {
            continue;
        }
        let value = p.value.as_ref()?;
        // Require a single text part — reject interpolated values like
        // `type="my-{x}"` where we can't statically resolve the type.
        let [svn_parser::AttrValuePart::Text { content, .. }] = value.parts.as_slice() else {
            return None;
        };
        return Some(content.as_str());
    }
    None
}
