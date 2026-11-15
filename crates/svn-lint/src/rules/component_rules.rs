//! Rules that fire on `<Component>` invocations.

use svn_parser::ast::{
    AttrValuePart, Attribute, Component, DirectiveKind, DirectiveValue, Fragment, Node,
};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::element_rules::{
    AttrParent, component_attribute_value_errors, sequence_expression_error,
    validate_component_slot_attribute, visit_attribute,
};

pub fn visit(comp: &Component, ctx: &mut LintContext<'_>) {
    check_component_attributes(&comp.attributes, ctx);
    for attr in &comp.attributes {
        let_directive_snippet_conflict(attr, &comp.attributes, &comp.children, ctx);
        visit_attribute(attr, &comp.attributes, ctx, AttrParent::Component);
    }
}

/// `let_directive_snippet_conflict` (`LetDirective.js`): a `let:`
/// directive on a component that has a `children` snippet as a direct
/// child. The snippet replaces the default slot content, so the
/// directive would never be given a value. A component placed in a
/// named slot (a `slot` attribute holding text) is exempt: there the
/// directive belongs to that slot.
pub(crate) fn let_directive_snippet_conflict(
    attr: &Attribute,
    attributes: &[Attribute],
    children: &Fragment,
    ctx: &mut LintContext<'_>,
) {
    let Attribute::Directive(directive) = attr else {
        return;
    };
    if directive.kind != DirectiveKind::Let {
        return;
    }
    // The compiler's `determine_slot`: the first `slot` attribute whose
    // value is one piece of text, when that text is not empty.
    let in_named_slot = attributes
        .iter()
        .find_map(|attribute| match attribute {
            Attribute::Plain(plain) if plain.name == "slot" => match plain.value.as_ref() {
                Some(value) => match value.parts.as_slice() {
                    [AttrValuePart::Text { range }] => Some(!range.slice(ctx.source).is_empty()),
                    [] if value.quoted => Some(false),
                    _ => None,
                },
                None => None,
            },
            _ => None,
        })
        .unwrap_or(false);
    if in_named_slot {
        return;
    }
    let has_children_snippet = children
        .nodes
        .iter()
        .any(|node| matches!(node, Node::SnippetBlock(snippet) if snippet.name == "children"));
    if !has_children_snippet {
        return;
    }
    // How the snippet parameter would be written: the name alone, or
    // `name: pattern` when the directive renames or destructures it.
    let expression = match &directive.value {
        Some(DirectiveValue::Expression {
            expression_range, ..
        }) => Some(expression_source(expression_range.slice(ctx.source))),
        _ => None,
    };
    let pattern = match expression {
        Some(text) if text != directive.name.as_str() => format!("{}: {text}", directive.name),
        _ => directive.name.to_string(),
    };
    ctx.emit_error(
        Code::let_directive_snippet_conflict,
        messages::let_directive_snippet_conflict(&pattern),
        directive.range,
    );
}

/// The text of the expression a directive's braces hold, as the
/// compiler's node for it spans: without parentheses around the whole
/// of it, and without comments before or after.
fn expression_source(text: &str) -> &str {
    use oxc_span::GetSpan;
    let allocator = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let Ok(parsed) = oxc_parser::Parser::new(&allocator, text, source_type).parse_expression()
    else {
        return text.trim();
    };
    let expression = parsed.without_parentheses();
    let span = expression.span();
    text.get(span.start as usize..span.end as usize)
        .unwrap_or(text.trim())
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
        if ctx.runes {
            component_attribute_value_errors(attr, ctx);
        }
        validate_component_slot_attribute(attr, ctx);
        // An attachment's expression is never an unparenthesized
        // comma sequence, whatever the mode.
        if let Attribute::Attach(a) = attr {
            sequence_expression_error(a.expression_range, ctx);
        }
    }
}
