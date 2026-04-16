//! Rune detection — the first semantic pass that proves the
//! no-character-level-scanning rule.
//!
//! Walks the script's oxc AST and records every call site of a Svelte 5
//! rune: `$state`, `$derived`, `$effect`, `$bindable`, `$inspect`, `$host`,
//! `$props`, plus dotted variants (`$state.raw`, `$derived.by`, `$effect.pre`,
//! `$effect.root`, `$state.snapshot`, `$props.id`).
//!
//! Deliberately AST-based. `-rs` had a ~800-line character scanner for this
//! and still got #1 (`parent$` truncation) and #2 (callable-store vs rune
//! misclassification) wrong. Pattern-matching on oxc `Expression` variants
//! makes both bugs categorically impossible: identifiers are identifiers, no
//! substring shenanigans.
//!
//! ### Scope of this pass
//!
//! - Top-level `VariableDeclaration` initializers
//! - Top-level `ExpressionStatement` expressions
//! - Recursive into `CallExpression` arguments, so nested rune calls like
//!   `$state({ inner: $derived(...) })` are found.
//!
//! Block statements (function bodies, loops) are NOT yet walked — runes
//! outside the top level of a Svelte script are almost always errors anyway,
//! and they're reported by the svelte compiler itself, not by us. We can
//! expand scope if real fixtures require it.

use oxc_ast::ast::{Argument, Expression, Program, Statement};
use svn_core::Range;

/// Every rune shape we recognize. Dotted variants have explicit variants so
/// downstream passes don't have to string-compare the tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RuneKind {
    State,
    StateRaw,
    StateSnapshot,
    Derived,
    DerivedBy,
    Effect,
    EffectPre,
    EffectRoot,
    Bindable,
    Inspect,
    InspectWith,
    Host,
    Props,
    PropsId,
}

impl RuneKind {
    /// Canonical textual spelling (for diagnostics).
    pub fn as_str(self) -> &'static str {
        match self {
            Self::State => "$state",
            Self::StateRaw => "$state.raw",
            Self::StateSnapshot => "$state.snapshot",
            Self::Derived => "$derived",
            Self::DerivedBy => "$derived.by",
            Self::Effect => "$effect",
            Self::EffectPre => "$effect.pre",
            Self::EffectRoot => "$effect.root",
            Self::Bindable => "$bindable",
            Self::Inspect => "$inspect",
            Self::InspectWith => "$inspect.with",
            Self::Host => "$host",
            Self::Props => "$props",
            Self::PropsId => "$props.id",
        }
    }
}

/// One detected rune call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuneCall {
    pub kind: RuneKind,
    /// Range covering the entire `CallExpression`, including arguments.
    pub range: Range,
    /// Range covering just the callee (`$state` or `$state.raw`).
    pub callee_range: Range,
}

/// Find every rune call in a script program.
pub fn find_runes(program: &Program<'_>) -> Vec<RuneCall> {
    let mut out = Vec::new();
    for stmt in &program.body {
        walk_statement(stmt, &mut out);
    }
    out
}

fn walk_statement(stmt: &Statement<'_>, out: &mut Vec<RuneCall>) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                if let Some(init) = &declarator.init {
                    walk_expression(init, out);
                }
            }
        }
        Statement::ExpressionStatement(es) => {
            walk_expression(&es.expression, out);
        }
        // We don't descend into other statement forms for rune detection —
        // runes outside top-level positions are compiler errors we let
        // Svelte report.
        _ => {}
    }
}

fn walk_expression(expr: &Expression<'_>, out: &mut Vec<RuneCall>) {
    if let Expression::CallExpression(call) = expr {
        if let Some(kind) = identify_rune_callee(&call.callee) {
            out.push(RuneCall {
                kind,
                range: Range::new(call.span.start, call.span.end),
                callee_range: callee_range(&call.callee),
            });
        }
        // Recurse into arguments so nested rune calls are caught.
        for arg in &call.arguments {
            walk_argument(arg, out);
        }
        walk_expression(&call.callee, out);
        return;
    }

    // For other expression shapes, recurse structurally into sub-expressions
    // that can hold rune calls.
    match expr {
        Expression::ArrayExpression(arr) => {
            for el in &arr.elements {
                if let Some(e) = el.as_expression() {
                    walk_expression(e, out);
                }
            }
        }
        Expression::ObjectExpression(obj) => {
            for prop in &obj.properties {
                if let oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) = prop {
                    walk_expression(&p.value, out);
                }
            }
        }
        Expression::ParenthesizedExpression(p) => walk_expression(&p.expression, out),
        Expression::SequenceExpression(seq) => {
            for e in &seq.expressions {
                walk_expression(e, out);
            }
        }
        Expression::ConditionalExpression(cond) => {
            walk_expression(&cond.test, out);
            walk_expression(&cond.consequent, out);
            walk_expression(&cond.alternate, out);
        }
        Expression::BinaryExpression(b) => {
            walk_expression(&b.left, out);
            walk_expression(&b.right, out);
        }
        Expression::LogicalExpression(l) => {
            walk_expression(&l.left, out);
            walk_expression(&l.right, out);
        }
        Expression::UnaryExpression(u) => walk_expression(&u.argument, out),
        Expression::AssignmentExpression(a) => walk_expression(&a.right, out),
        // Member expressions can't be runes unless they're the callee of a
        // CallExpression (handled above). Their object can still host
        // nested call expressions though.
        Expression::StaticMemberExpression(m) => walk_expression(&m.object, out),
        Expression::ComputedMemberExpression(m) => {
            walk_expression(&m.object, out);
            walk_expression(&m.expression, out);
        }
        // Literal types and identifiers terminate the walk.
        _ => {}
    }
}

fn walk_argument(arg: &Argument<'_>, out: &mut Vec<RuneCall>) {
    match arg {
        Argument::SpreadElement(se) => walk_expression(&se.argument, out),
        other => {
            if let Some(e) = other.as_expression() {
                walk_expression(e, out);
            }
        }
    }
}

fn identify_rune_callee(callee: &Expression<'_>) -> Option<RuneKind> {
    match callee {
        Expression::Identifier(id) => match id.name.as_str() {
            "$state" => Some(RuneKind::State),
            "$derived" => Some(RuneKind::Derived),
            "$effect" => Some(RuneKind::Effect),
            "$bindable" => Some(RuneKind::Bindable),
            "$inspect" => Some(RuneKind::Inspect),
            "$host" => Some(RuneKind::Host),
            "$props" => Some(RuneKind::Props),
            _ => None,
        },
        Expression::StaticMemberExpression(m) => {
            let Expression::Identifier(obj) = &m.object else {
                return None;
            };
            let prop = m.property.name.as_str();
            match (obj.name.as_str(), prop) {
                ("$state", "raw") => Some(RuneKind::StateRaw),
                ("$state", "snapshot") => Some(RuneKind::StateSnapshot),
                ("$derived", "by") => Some(RuneKind::DerivedBy),
                ("$effect", "pre") => Some(RuneKind::EffectPre),
                ("$effect", "root") => Some(RuneKind::EffectRoot),
                ("$inspect", "with") => Some(RuneKind::InspectWith),
                ("$props", "id") => Some(RuneKind::PropsId),
                _ => None,
            }
        }
        _ => None,
    }
}

fn callee_range(callee: &Expression<'_>) -> Range {
    match callee {
        Expression::Identifier(id) => Range::new(id.span.start, id.span.end),
        Expression::StaticMemberExpression(m) => Range::new(m.span.start, m.span.end),
        other => {
            // Shouldn't happen for valid rune callees, but be defensive.
            let span = oxc_span::GetSpan::span(other);
            Range::new(span.start, span.end)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::{ScriptLang, parse_script_body};

    fn runes_in(src: &str) -> Vec<RuneKind> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        assert!(
            parsed.errors.is_empty(),
            "parse errors: {:?}",
            parsed.errors
        );
        find_runes(&parsed.program)
            .into_iter()
            .map(|r| r.kind)
            .collect()
    }

    #[test]
    fn no_runes_in_plain_code() {
        assert_eq!(runes_in("let x = 1;"), Vec::<RuneKind>::new());
        assert_eq!(
            runes_in("import { foo } from 'bar';"),
            Vec::<RuneKind>::new()
        );
    }

    #[test]
    fn finds_basic_state() {
        assert_eq!(runes_in("let x = $state(0);"), vec![RuneKind::State]);
    }

    #[test]
    fn finds_all_basic_runes() {
        let src = r#"
            let a = $state(0);
            let b = $derived(a + 1);
            let c = $bindable(0);
            let d = $props();
            $effect(() => {});
            $inspect(a);
            let h = $host();
        "#;
        let runes = runes_in(src);
        assert!(runes.contains(&RuneKind::State));
        assert!(runes.contains(&RuneKind::Derived));
        assert!(runes.contains(&RuneKind::Bindable));
        assert!(runes.contains(&RuneKind::Props));
        assert!(runes.contains(&RuneKind::Effect));
        assert!(runes.contains(&RuneKind::Inspect));
        assert!(runes.contains(&RuneKind::Host));
    }

    #[test]
    fn finds_dotted_variants() {
        let src = r#"
            let raw = $state.raw({});
            let snap = $state.snapshot(x);
            let by = $derived.by(() => 1);
            $effect.pre(() => {});
            $effect.root(() => {});
            $inspect.with(x, console.log);
            let id = $props.id();
        "#;
        let runes = runes_in(src);
        assert!(runes.contains(&RuneKind::StateRaw));
        assert!(runes.contains(&RuneKind::StateSnapshot));
        assert!(runes.contains(&RuneKind::DerivedBy));
        assert!(runes.contains(&RuneKind::EffectPre));
        assert!(runes.contains(&RuneKind::EffectRoot));
        assert!(runes.contains(&RuneKind::InspectWith));
        assert!(runes.contains(&RuneKind::PropsId));
    }

    #[test]
    fn dollar_suffix_identifier_not_a_rune() {
        // `parent$` is an identifier that happens to start with `parent` and
        // end with `$`. It is NOT `$parent`. bug #1 from the -rs rescue —
        // a char-level scanner misclassified this.
        let src = "const parent$ = writable(0);";
        assert_eq!(runes_in(src), Vec::<RuneKind>::new());
    }

    #[test]
    fn callable_store_not_a_rune() {
        // svelte-i18n exposes `t` as a store, auto-subscribed as `$t`. At
        // the script AST level this shows up as a normal function call to an
        // identifier named `$t` — but `$t` is not a rune. bug #2 from the
        // -rs rescue. Our whitelist in identify_rune_callee excludes it.
        let src = r#"
            import { t } from 'svelte-i18n';
            const label = $t('hello');
        "#;
        assert_eq!(runes_in(src), Vec::<RuneKind>::new());
    }

    #[test]
    fn rune_in_string_literal_is_not_a_rune() {
        // Character-level scanners used to misflag this. oxc sees it as a
        // StringLiteral and never walks into it.
        let src = r#"let msg = "use $state() in components";"#;
        assert_eq!(runes_in(src), Vec::<RuneKind>::new());
    }

    #[test]
    fn rune_in_comment_is_not_a_rune() {
        let src = r#"
            // use $state here
            /* $derived is also a thing */
            let x = 1;
        "#;
        assert_eq!(runes_in(src), Vec::<RuneKind>::new());
    }

    #[test]
    fn nested_rune_in_argument_is_caught() {
        // $state({ inner: $derived(...) }) — the outer walker should descend
        // into the argument object.
        let src = "let outer = $state({ inner: $derived(0) });";
        let runes = runes_in(src);
        assert!(runes.contains(&RuneKind::State));
        assert!(runes.contains(&RuneKind::Derived));
    }

    #[test]
    fn rune_in_conditional_expression() {
        let src = "let x = cond ? $state(0) : $derived(1);";
        let runes = runes_in(src);
        assert!(runes.contains(&RuneKind::State));
        assert!(runes.contains(&RuneKind::Derived));
    }

    #[test]
    fn range_covers_full_call_expression() {
        let alloc = Allocator::default();
        let src = "let x = $state(0);";
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        let runes = find_runes(&parsed.program);
        assert_eq!(runes.len(), 1);
        let r = &runes[0];
        // "$state(0)" starts at index 8, ends at index 17 (exclusive).
        assert_eq!(r.range.slice(src), "$state(0)");
        assert_eq!(r.callee_range.slice(src), "$state");
    }

    #[test]
    fn kind_as_str_matches_canonical() {
        assert_eq!(RuneKind::State.as_str(), "$state");
        assert_eq!(RuneKind::DerivedBy.as_str(), "$derived.by");
        assert_eq!(RuneKind::EffectRoot.as_str(), "$effect.root");
    }

    #[test]
    fn unknown_dotted_is_not_a_rune() {
        // `$state.foo()` is not a real rune — should not be flagged.
        let src = "let x = $state.foo();";
        let runes = runes_in(src);
        assert!(
            !runes
                .iter()
                .any(|r| matches!(r, RuneKind::State | RuneKind::StateRaw))
        );
    }

    #[test]
    fn shadowed_rune_identifier_still_matched() {
        // If a user declares a local named `$state`, our AST-level detection
        // still flags calls as runes — the svelte compiler is the one that
        // catches shadowing. This test documents the current behavior.
        let src = r#"
            const $state = (x: number) => x + 1;
            const y = $state(2);
        "#;
        let runes = runes_in(src);
        assert!(runes.contains(&RuneKind::State));
        // Note: that might be incorrect semantically but it matches what
        // Svelte 5 itself considers a rune (identifier-based). Either way
        // we don't own the "shadowing is illegal" rule — the Svelte
        // compiler does.
    }
}
