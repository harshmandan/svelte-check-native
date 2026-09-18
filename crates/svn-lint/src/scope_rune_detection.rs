//! Rune-call detection helpers.
//!
//! Pure functions over `oxc_ast` expressions — no [`crate::scope::ScopeTree`]
//! state access. Pulled out of `scope.rs` so the rune-detection
//! surface (one upstream-aligned predicate per rune) reads as a
//! single small module instead of being scattered between two
//! impl blocks.

use oxc_ast::ast::{BindingPattern, CallExpression, Expression};

use smol_str::SmolStr;

use crate::scope_types::{InitialKind, RuneCall, StateArg};
use crate::scope_util::unwrap_ts_wrappers;

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
            primitive_arg: StateArg::Primitive,
            ..
        }
    )
}

/// For a `$state`/`$state.raw` call init, classify the argument the
/// way `Identifier.js` does before asking `should_proxy`: only a call
/// with exactly one non-spread argument can be primitive.
pub(crate) fn state_rune_primitive_arg(e: &Expression<'_>) -> StateArg {
    let Expression::CallExpression(c) = e else {
        return StateArg::Proxied;
    };
    let [arg] = c.arguments.as_slice() else {
        return StateArg::Proxied;
    };
    let Some(arg) = arg.as_expression() else {
        return StateArg::Proxied;
    };
    let arg = unwrap_ts_wrappers(arg);
    if is_primitive_expr(arg) {
        StateArg::Primitive
    } else if let Expression::Identifier(id) = arg {
        StateArg::Ident(SmolStr::from(id.name.as_str()))
    } else {
        StateArg::Proxied
    }
}

pub(crate) fn detect_rune_call_from_call(c: &CallExpression<'_>) -> Option<RuneCall> {
    Some(match &c.callee {
        Expression::Identifier(id) => match id.name.as_str() {
            "$state" => RuneCall::State,
            "$derived" => RuneCall::Derived,
            "$props" => RuneCall::Props,
            "$bindable" => RuneCall::Bindable,
            "$inspect" => RuneCall::Inspect,
            "$host" => RuneCall::Host,
            "$effect" => RuneCall::Effect,
            _ => return None,
        },
        Expression::StaticMemberExpression(m) => {
            let Expression::Identifier(o) = &m.object else {
                return None;
            };
            match (o.name.as_str(), m.property.name.as_str()) {
                ("$state", "raw") => RuneCall::StateRaw,
                ("$derived", "by") => RuneCall::DerivedBy,
                _ => return None,
            }
        }
        _ => return None,
    })
}

/// Detects `$bindable(default)` inside a $props() destructure default
/// position. Returns `Some(primitive)` where primitive is whether the
/// arg is a primitive-literal-ish thing, or `None` if not a $bindable
/// call.
pub(crate) fn detect_bindable_default(pat: &BindingPattern<'_>) -> Option<bool> {
    match pat {
        BindingPattern::AssignmentPattern(ap) => match &ap.right {
            Expression::CallExpression(c) => {
                if detect_rune_call_from_call(c) == Some(RuneCall::Bindable) {
                    let arg_is_primitive = c
                        .arguments
                        .first()
                        .and_then(|a| a.as_expression())
                        .map(|arg| is_primitive_expr(unwrap_ts_wrappers(arg)))
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
            | Expression::RegExpLiteral(_)
            | Expression::TemplateLiteral(_)
            | Expression::ArrowFunctionExpression(_)
            | Expression::FunctionExpression(_)
            | Expression::UnaryExpression(_)
            | Expression::BinaryExpression(_)
    ) || matches!(e, Expression::Identifier(id) if id.name.as_str() == "undefined")
}

/// The rune a call's callee names — the compiler's
/// `get_global_keypath`: a chain of non-computed member accesses on an
/// identifier, where a call on the identifier reads as `()`
/// (`$inspect(x).with` → `$inspect().with`). Returns the keypath when
/// it is a rune, with the root identifier's name. Whether the root
/// resolves to a binding (which makes it no rune) is the caller's
/// question.
pub(crate) fn rune_keypath(callee: &Expression<'_>) -> Option<(String, String)> {
    let mut suffix = String::new();
    let mut n = unwrap_ts_wrappers(callee);
    while let Expression::StaticMemberExpression(m) = n {
        suffix.insert_str(0, m.property.name.as_str());
        suffix.insert(0, '.');
        n = unwrap_ts_wrappers(&m.object);
    }
    if matches!(
        n,
        Expression::ComputedMemberExpression(_) | Expression::PrivateFieldExpression(_)
    ) {
        return None;
    }
    if let Expression::CallExpression(c) = n
        && let Expression::Identifier(_) = unwrap_ts_wrappers(&c.callee)
    {
        suffix.insert_str(0, "()");
        n = unwrap_ts_wrappers(&c.callee);
    }
    let Expression::Identifier(id) = n else {
        return None;
    };
    let keypath = format!("{}{suffix}", id.name);
    is_rune_name(&keypath).then(|| (keypath, id.name.to_string()))
}
