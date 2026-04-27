//! Rune-call detection helpers.
//!
//! Pure functions over `oxc_ast` expressions — no [`crate::scope::ScopeTree`]
//! state access. Pulled out of `scope.rs` so the rune-detection
//! surface (one upstream-aligned predicate per rune) reads as a
//! single small module instead of being scattered between two
//! impl blocks.

use oxc_ast::ast::{BindingPattern, BindingPatternKind, CallExpression, Expression};

use crate::scope_types::{InitialKind, RuneCall};

/// Matches upstream `utils.js::is_rune`. Keep in sync with the
/// `RUNES` constant there.
pub fn is_rune_name(name: &str) -> bool {
    matches!(
        name,
        "$state"
            | "$state.raw"
            | "$state.eager"
            | "$state.snapshot"
            | "$derived"
            | "$derived.by"
            | "$props"
            | "$props.id"
            | "$bindable"
            | "$effect"
            | "$effect.pre"
            | "$effect.tracking"
            | "$effect.root"
            | "$effect.pending"
            | "$inspect"
            | "$inspect().with"
            | "$inspect.trace"
            | "$host"
    )
}

/// Was this binding declared with a `$state(primitive)`-style init?
/// The `InitialKind::RuneCall.primitive_arg` flag captures this —
/// true for `$state(0)`, `$state.raw(0)`, false for `$state({})`.
pub(crate) fn is_primitive_rune_init(init: &InitialKind) -> bool {
    matches!(
        init,
        InitialKind::RuneCall {
            primitive_arg: true,
            ..
        }
    )
}

/// For a `$state`/`$state.raw` call init, return whether the first
/// argument is a primitive-like (matching upstream's `should_proxy`
/// analog). `true` if no argument.
pub(crate) fn state_rune_primitive_arg(e: &Expression<'_>) -> bool {
    if let Expression::CallExpression(c) = e {
        c.arguments
            .first()
            .and_then(|a| a.as_expression())
            .map(is_primitive_expr)
            .unwrap_or(true)
    } else {
        true
    }
}

pub(crate) fn detect_rune_call_from_call(c: &CallExpression<'_>) -> Option<RuneCall> {
    let callee_name = match &c.callee {
        Expression::Identifier(id) => id.name.as_str().to_string(),
        Expression::StaticMemberExpression(m) => {
            if let Expression::Identifier(o) = &m.object {
                format!("{}.{}", o.name.as_str(), m.property.name.as_str())
            } else {
                return None;
            }
        }
        _ => return None,
    };
    Some(match callee_name.as_str() {
        "$state" => RuneCall::State,
        "$state.raw" => RuneCall::StateRaw,
        "$derived" => RuneCall::Derived,
        "$derived.by" => RuneCall::DerivedBy,
        "$props" => RuneCall::Props,
        "$bindable" => RuneCall::Bindable,
        "$inspect" => RuneCall::Inspect,
        "$host" => RuneCall::Host,
        "$effect" => RuneCall::Effect,
        _ => return None,
    })
}

/// Detects `$bindable(default)` inside a $props() destructure default
/// position. Returns `Some(primitive)` where primitive is whether the
/// arg is a primitive-literal-ish thing, or `None` if not a $bindable
/// call.
pub(crate) fn detect_bindable_default(pat: &BindingPattern<'_>) -> Option<bool> {
    match &pat.kind {
        BindingPatternKind::AssignmentPattern(ap) => match &ap.right {
            Expression::CallExpression(c) => {
                if detect_rune_call_from_call(c) == Some(RuneCall::Bindable) {
                    let arg_is_primitive = c
                        .arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .map(is_primitive_expr)
                        .unwrap_or(true);
                    Some(arg_is_primitive)
                } else {
                    None
                }
            }
            _ => None,
        },
        _ => None,
    }
}

/// Conservative `should_proxy`-analog — upstream
/// `3-transform/client/utils.js::should_proxy`. Returns `true` if the
/// expression is one of the primitive-like kinds that should NOT be
/// proxied.
pub(crate) fn is_primitive_expr(e: &Expression<'_>) -> bool {
    matches!(
        e,
        Expression::NullLiteral(_)
            | Expression::NumericLiteral(_)
            | Expression::StringLiteral(_)
            | Expression::BooleanLiteral(_)
            | Expression::BigIntLiteral(_)
            | Expression::TemplateLiteral(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::UnaryExpression(_)
            | Expression::BinaryExpression(_)
    ) || matches!(e, Expression::Identifier(id) if id.name.as_str() == "undefined")
}
