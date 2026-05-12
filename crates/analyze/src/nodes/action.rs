//! `use:` directive analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/Action.ts`.

use smol_str::SmolStr;
use svn_parser::{Directive, DirectiveValue};

use crate::walker::{ActionDirective, Counters, TemplateSummary};

/// Handle the `use:` arm of `walk_directive`. Registers a
/// `__svn_action_attrs_N` void-ref (one per directive, counter
/// shared workspace-wide per component) and captures the action's
/// full shape (name, host tag, params range) so emit can build
/// the real `action(element, params)` call rather than the
/// pre-v0.3.9 placeholder that dropped both sides and lost
/// contextual typing on the params expression.
pub(crate) fn handle_use_directive(
    d: &Directive,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    parent_tag: Option<&str>,
) {
    let index = counters.action_attrs;
    let name = format!("__svn_action_attrs_{index}");
    summary.void_refs.register(name);
    counters.action_attrs += 1;
    let params_range = match &d.value {
        Some(DirectiveValue::Expression {
            expression_range, ..
        }) => Some(*expression_range),
        _ => None,
    };
    summary.action_directives.push(ActionDirective {
        index,
        action_name: d.name.clone(),
        tag_name: parent_tag.map(SmolStr::new),
        params_range,
    });
}
