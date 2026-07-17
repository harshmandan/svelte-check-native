//! JS/TS AST rules that fire inside `<script>` blocks.
//!
//! There is no traversal here: the scope builder's `ScriptWalker`
//! (`crate::scope`) is the ONE walk over each script body, and it
//! drives these rules through the [`ScriptRuleHooks`] callbacks at
//! the node kinds the rules care about. The hooks don't emit
//! directly — the scope build runs before the `LintContext` is ready
//! for warnings (and may run twice when the provisional runes answer
//! flips, discarding the first tree) — so each hook buffers a
//! [`ScriptRuleEvent`] on the tree builder. [`flush`] replays the
//! buffer at the pipeline stage where these rules have always
//! emitted: after the `<svelte:options>` attribute warnings, before
//! the walk-time binding rules. Buffer order is walk order (module
//! script first, then instance — upstream's analyze order), so the
//! user-visible warning order is unchanged.
//!
//! Upstream equivalents:
//! - `perf_avoid_inline_class` → `visitors/NewExpression.js:11`
//! - `perf_avoid_nested_class` → `visitors/ClassDeclaration.js:21`
//! - `reactive_declaration_invalid_placement` → `visitors/LabeledStatement.js:90`
//! - `bidirectional_control_characters` → the Literal / TemplateLiteral visitors
//! - `legacy_component_creation` → `visitors/ExpressionStatement.js`

use oxc_ast::ast::{Expression, LabeledStatement, NewExpression, TemplateLiteral};
use smol_str::SmolStr;
use svn_core::Range;

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// One buffered rule outcome from the shared script walk.
pub(crate) enum ScriptRuleEvent {
    /// A warning fully decided at walk time.
    Warning {
        code: Code,
        message: String,
        range: Range,
    },
    /// `new Callee({ target: … })` — the syntactic half of
    /// `legacy_component_creation`, matched at walk time. Whether it
    /// fires depends on `callee` resolving to a default import from a
    /// `.svelte` source, which needs the FINISHED scope tree — so the
    /// resolution happens in [`flush`], exactly when the old
    /// standalone pass ran.
    LegacyCreationCandidate { callee: SmolStr, range: Range },
}

/// Per-script-walk configuration for the rule hooks. Carried as
/// `Option<ScriptRuleHooks>` by the scope builder's `ScriptWalker`:
/// `Some` for the module / instance script walks, `None` for the
/// template mini-expression walks (these rules never ran on template
/// expressions).
///
/// The walker's `function_depth` convention matches upstream's
/// analyze-phase convention byte-for-byte — the module root scope has
/// depth 0 and the instance root depth 1 (the implicit component
/// function) — so hooks read the walker's counter directly. The
/// walker's `rune_bump` (a `$derived(…)`-argument elevation applied
/// only to recorded references) is deliberately NOT included:
/// upstream's perf visitors consult `scope.function_depth`, which the
/// rune bump never touches.
#[derive(Clone, Copy)]
pub(crate) struct ScriptRuleHooks {
    /// The file's runes mode. The retained scope tree is always the
    /// one built under the FINAL runes answer (a flip triggers a
    /// rebuild that discards the first buffer), so this matches
    /// `ctx.runes` at flush time.
    pub runes: bool,
    /// Instance script vs module script.
    pub is_instance: bool,
}

/// Does any active ignore frame mention `code`? Upstream's per-node
/// stack union — an ignore above a statement suppresses warnings
/// anchored anywhere in the subtree.
fn is_ignored(frames: &[Vec<SmolStr>], code: Code) -> bool {
    frames
        .iter()
        .any(|frame| frame.iter().any(|c| c.as_str() == code.as_str()))
}

/// Matches upstream's `regex_bidirectional_control_characters`.
fn has_bidi_char(s: &str) -> bool {
    s.chars()
        .any(|c| matches!(c as u32, 0x202A..=0x202E | 0x2066..=0x2069))
}

impl ScriptRuleHooks {
    /// Class-declaration statement (named or exported).
    ///
    /// `perf_avoid_nested_class`: runes mode only. Upstream
    /// (`visitors/ClassDeclaration.js:21`):
    ///   allowed_depth = ast_type === 'module' ? 0 : 1;
    ///   if (scope.function_depth > allowed_depth) w.perf_avoid_nested_class(node);
    /// Exported class declarations are syntactically top-level, so
    /// they sit exactly at the allowed depth and can never trip the
    /// check — running it uniformly for every class-declaration
    /// statement is safe.
    pub fn class_declaration(
        &self,
        events: &mut Vec<ScriptRuleEvent>,
        frames: &[Vec<SmolStr>],
        function_depth: u32,
        range: Range,
    ) {
        if !self.runes {
            return;
        }
        let allowed = if self.is_instance { 1 } else { 0 };
        if function_depth > allowed && !is_ignored(frames, Code::perf_avoid_nested_class) {
            events.push(ScriptRuleEvent::Warning {
                code: Code::perf_avoid_nested_class,
                message: messages::perf_avoid_nested_class(),
                range,
            });
        }
    }

    /// `perf_avoid_inline_class`: `new (class {…})` at any
    /// function_depth > 0. Upstream (`visitors/NewExpression.js:11`)
    /// fires only when `callee` is a ClassExpression.
    pub fn new_expression(
        &self,
        events: &mut Vec<ScriptRuleEvent>,
        frames: &[Vec<SmolStr>],
        function_depth: u32,
        ne: &NewExpression<'_>,
        range: Range,
    ) {
        if function_depth > 0
            && matches!(&ne.callee, Expression::ClassExpression(_))
            && !is_ignored(frames, Code::perf_avoid_inline_class)
        {
            events.push(ScriptRuleEvent::Warning {
                code: Code::perf_avoid_inline_class,
                message: messages::perf_avoid_inline_class(),
                range,
            });
        }
    }

    /// `reactive_declaration_invalid_placement`: a `$:` label that's
    /// not a DIRECT child of the INSTANCE script's Program. Upstream
    /// compares the label's parent against Program
    /// (`LabeledStatement.js:90`), so a bare block or if body at
    /// depth 0 still fires (verified against the compiler). Fires
    /// outside runes mode only — the error path inside runes handles
    /// the label instead.
    pub fn labeled_statement(
        &self,
        events: &mut Vec<ScriptRuleEvent>,
        frames: &[Vec<SmolStr>],
        at_program_top: bool,
        lbl: &LabeledStatement<'_>,
        range: Range,
    ) {
        if lbl.label.name != "$" {
            return;
        }
        let is_reactive_statement = self.is_instance && at_program_top;
        if !self.runes
            && !is_reactive_statement
            && !is_ignored(frames, Code::reactive_declaration_invalid_placement)
        {
            events.push(ScriptRuleEvent::Warning {
                code: Code::reactive_declaration_invalid_placement,
                message: messages::reactive_declaration_invalid_placement(),
                range,
            });
        }
    }

    /// `bidirectional_control_characters` on a string literal.
    pub fn string_literal(
        &self,
        events: &mut Vec<ScriptRuleEvent>,
        frames: &[Vec<SmolStr>],
        value: &str,
        range: Range,
    ) {
        if has_bidi_char(value) && !is_ignored(frames, Code::bidirectional_control_characters) {
            events.push(ScriptRuleEvent::Warning {
                code: Code::bidirectional_control_characters,
                message: messages::bidirectional_control_characters(),
                range,
            });
        }
    }

    /// `bidirectional_control_characters` on a template literal's
    /// quasis. All quasis are checked before the caller walks the
    /// interpolation expressions, matching upstream's visitor order.
    pub fn template_literal(
        &self,
        events: &mut Vec<ScriptRuleEvent>,
        frames: &[Vec<SmolStr>],
        tl: &TemplateLiteral<'_>,
        base_offset: u32,
    ) {
        for q in &tl.quasis {
            if let Some(cooked) = q.value.cooked.as_deref()
                && has_bidi_char(cooked)
                && !is_ignored(frames, Code::bidirectional_control_characters)
            {
                events.push(ScriptRuleEvent::Warning {
                    code: Code::bidirectional_control_characters,
                    message: messages::bidirectional_control_characters(),
                    range: Range::new(q.span.start + base_offset, q.span.end + base_offset),
                });
            }
        }
    }

    /// Upstream `visitors/ExpressionStatement.js` — the Svelte 4
    /// class-instantiation pattern `new ComponentName({ target: … })`
    /// that no longer works in Svelte 5. Fires from expression
    /// STATEMENTS only, so `throw new App({target})` and
    /// `const app = new App({target})` do not warn (verified against
    /// the compiler). The callee-resolution half runs at flush time.
    pub fn expression_statement(
        &self,
        events: &mut Vec<ScriptRuleEvent>,
        frames: &[Vec<SmolStr>],
        expr: &Expression<'_>,
        base_offset: u32,
    ) {
        let Some((callee, span)) = legacy_creation_candidate(expr) else {
            return;
        };
        if is_ignored(frames, Code::legacy_component_creation) {
            return;
        }
        events.push(ScriptRuleEvent::LegacyCreationCandidate {
            callee: SmolStr::from(callee),
            range: Range::new(span.start + base_offset, span.end + base_offset),
        });
    }
}

/// The syntactic half of `legacy_component_creation`: a `new` on a
/// bare identifier with exactly one argument — an ObjectExpression
/// carrying a `target` property. `new Foo()` and `new Foo({})`
/// deliberately skip.
fn legacy_creation_candidate<'a>(expr: &'a Expression<'_>) -> Option<(&'a str, oxc_span::Span)> {
    let Expression::NewExpression(ne) = expr else {
        return None;
    };
    let Expression::Identifier(callee) = &ne.callee else {
        return None;
    };
    if ne.arguments.len() != 1 {
        return None;
    }
    let arg = ne.arguments[0].as_expression()?;
    let Expression::ObjectExpression(obj) = arg else {
        return None;
    };
    let has_target = obj.properties.iter().any(|p| {
        matches!(
            p,
            oxc_ast::ast::ObjectPropertyKind::ObjectProperty(op)
                if matches!(&op.key, oxc_ast::ast::PropertyKey::StaticIdentifier(k) if k.name.as_str() == "target")
        )
    });
    has_target.then(|| (callee.name.as_str(), ne.span))
}

/// Replay the buffered events in walk order. Runs at the pipeline
/// stage where this module's standalone walk used to emit, with the
/// finished scope tree available on `ctx` for the
/// `legacy_component_creation` callee resolution: the callee must be
/// a default import from a `.svelte` source.
pub(crate) fn flush(events: Vec<ScriptRuleEvent>, ctx: &mut LintContext<'_>) {
    for event in events {
        match event {
            ScriptRuleEvent::Warning {
                code,
                message,
                range,
            } => ctx.emit(code, message, range),
            ScriptRuleEvent::LegacyCreationCandidate { callee, range } => {
                let Some(tree) = &ctx.scope_tree else {
                    continue;
                };
                let Some(bid) = tree.resolve_from_template(&callee) else {
                    continue;
                };
                let is_svelte_default_import = matches!(
                    &tree.binding(bid).initial,
                    crate::scope::InitialKind::Import { source, is_default: true }
                        if source.ends_with(".svelte")
                );
                if is_svelte_default_import {
                    ctx.emit(
                        Code::legacy_component_creation,
                        messages::legacy_component_creation(),
                        range,
                    );
                }
            }
        }
    }
}
