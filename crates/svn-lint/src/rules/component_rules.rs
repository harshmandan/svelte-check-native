//! Rules that fire on `<Component>` invocations.

use svn_parser::ast::{Attribute, Component, DirectiveKind};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::element_rules::{AttrParent, validate_component_slot_attribute, visit_attribute};

pub fn visit(comp: &Component, ctx: &mut LintContext<'_>) {
    check_component_attributes(&comp.attributes, ctx);
    for attr in &comp.attributes {
        visit_attribute(attr, &comp.attributes, ctx, AttrParent::Component);
    }
}

/// The compiler's first pass over a component's attributes
/// (`visit_component`, shared by `<Component>`, `<svelte:component>`
/// and `<svelte:self>`). It checks every attribute before any
/// attribute is visited on its own, so its errors come before the
/// errors those visits raise, whatever the attribute order:
///
/// - a directive other than `let:`, `on:` or `bind:` is an error
///   (attachments and spreads are fine);
/// - an `on:` directive may carry no modifier but a single `once`;
/// - a `slot` attribute must have a static value.
pub(crate) fn check_component_attributes(attributes: &[Attribute], ctx: &mut LintContext<'_>) {
    for attr in attributes {
        if let Attribute::Directive(d) = attr {
            match d.kind {
                DirectiveKind::Let | DirectiveKind::Bind => {}
                DirectiveKind::On => {
                    if d.modifiers.len() > 1 || d.modifiers.iter().any(|m| m != "once") {
                        ctx.emit_error(
                            Code::event_handler_invalid_component_modifier,
                            messages::event_handler_invalid_component_modifier(),
                            d.range,
                        );
                    }
                }
                DirectiveKind::Use
                | DirectiveKind::Class
                | DirectiveKind::Style
                | DirectiveKind::Transition
                | DirectiveKind::In
                | DirectiveKind::Out
                | DirectiveKind::Animate => ctx.emit_error(
                    Code::component_invalid_directive,
                    messages::component_invalid_directive(),
                    d.range,
                ),
            }
        }
        validate_component_slot_attribute(attr, ctx);
    }
}
