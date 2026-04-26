//! Template-scope pattern primitive — shared between analyze and lint.
//!
//! Both `analyze::template_walker` and `svn-lint::scope` walk the same
//! kind of binding pattern (each-block context, snippet params, let
//! directive payload, `{@const}` declarator) and reach the same
//! conclusions about which identifiers are bound. Each round of bug
//! fixes had to land in both — F5 (`{:then}`/`{:catch}` branch
//! isolation), G6 (`{@const}` retag boundary), G9 (array-rest
//! coverage). The plan in `notes/PLAN-template-scope-unification.md`
//! drives consolidation behind a single primitive; this module is
//! Phase 1: the leaf pattern walker that both crates call.
//!
//! ### What lives here today
//!
//! - [`BoundIdent`] — name + source span + `inside_rest` flag.
//! - [`PatternBindings`] — the helper's output: the bindings emitted
//!   in walk order, plus the default-value expression ranges that
//!   the caller is responsible for walking in PARENT scope (they're
//!   read-side, not declared).
//! - [`collect_pattern_bindings`] — pre-parsed-pattern entry point.
//!   Caller picks the right oxc wrapper for the construct it's
//!   handling (`(slice) => 0`, `let X;`, `function _(X) {}`) and
//!   passes the resulting `BindingPattern` plus an offset that
//!   translates oxc spans back to the original source.
//!
//! ### What does NOT live here yet
//!
//! Phase 1's scope is only the pattern walker. The full visitor trait
//! (with `enter_scope` / `leave_scope` / `visit_at_const` hooks) and
//! the unified template walker arrive in Phases 3-4 of the plan. See
//! `notes/PLAN-template-scope-unification.md`.

use oxc_ast::ast::{BindingPattern, BindingPatternKind};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_core::Range;

/// One identifier bound by a destructure / params / `{@const}` /
/// let-directive pattern. Coordinates are in the ORIGINAL source
/// (after the caller's parser-wrapper offset has been applied).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundIdent {
    pub name: SmolStr,
    pub range: Range,
    /// `true` when this identifier sits inside a rest element
    /// (`...rest` for objects; `[..., ...rest]` for arrays). Lint's
    /// `bind_invalid_each_rest` rule consumes the flag; analyze
    /// ignores it.
    pub inside_rest: bool,
}

/// Output of [`collect_pattern_bindings`]. Bindings are emitted in
/// walk order — same order as the original recursive callers
/// declared them — so callers iterating sequentially produce
/// byte-equivalent results to the pre-helper recursion.
///
/// `default_value_ranges` carries the source spans of every
/// `AssignmentPattern`'s right-hand expression. These are NOT
/// declarations — the caller (lint) walks them as expressions in
/// the PARENT scope so a `{ a = b }` default's `b` resolves to a
/// parent binding rather than the just-declared `a`.
#[derive(Debug, Default, Clone)]
pub struct PatternBindings {
    pub bindings: Vec<BoundIdent>,
    pub default_value_ranges: Vec<Range>,
}

/// Walk a pre-parsed `BindingPattern` and return every identifier it
/// introduces, plus the source ranges of any default-value expressions
/// for the caller to walk separately.
///
/// `offset` translates oxc spans (relative to the wrapper-text the
/// caller parsed) back to the original source. For example, if the
/// caller wrapped a slice as `let X = 0;` (4 bytes of prefix) and
/// the pattern slice starts at byte 12 in the original source, the
/// offset is `12 - 4 = 8`.
///
/// `inside_rest` is the inherited flag — `false` at the root, set to
/// `true` when descending into a rest binding (`{ ...rest }` or
/// `[..., ...rest]`).
pub fn collect_pattern_bindings(pat: &BindingPattern<'_>, offset: i32) -> PatternBindings {
    let mut out = PatternBindings::default();
    walk(pat, offset, false, &mut out);
    out
}

fn walk(pat: &BindingPattern<'_>, offset: i32, inside_rest: bool, out: &mut PatternBindings) {
    match &pat.kind {
        BindingPatternKind::BindingIdentifier(id) => {
            let start = (id.span.start as i32 + offset).max(0) as u32;
            let end = (id.span.end as i32 + offset).max(0) as u32;
            out.bindings.push(BoundIdent {
                name: SmolStr::from(id.name.as_str()),
                range: Range::new(start, end),
                inside_rest,
            });
        }
        BindingPatternKind::ObjectPattern(op) => {
            for prop in &op.properties {
                walk(&prop.value, offset, inside_rest, out);
            }
            // Object-rest (`{ ...rest }`) — lint flags this for
            // `bind_invalid_each_rest`; analyze just needs the name
            // for shadow tracking. Either way, the inside-rest flag
            // propagates to anything inside.
            if let Some(rest) = &op.rest {
                walk(&rest.argument, offset, true, out);
            }
        }
        BindingPatternKind::ArrayPattern(ap) => {
            for elem in ap.elements.iter().flatten() {
                walk(elem, offset, inside_rest, out);
            }
            // Array-rest (`[head, ...tail]`). Round-4 G9: must walk
            // here, otherwise `tail` never reaches the shadow stack
            // and template references resolve to a wrong binding.
            if let Some(rest) = &ap.rest {
                walk(&rest.argument, offset, true, out);
            }
        }
        BindingPatternKind::AssignmentPattern(asn) => {
            // The pattern's left side is the binding(s); the right
            // side is the default-value expression. Lint walks the
            // default in PARENT scope (refs resolve to outer
            // bindings), so we just emit its range and let the caller
            // decide.
            walk(&asn.left, offset, inside_rest, out);
            let right_span = asn.right.span();
            let start = (right_span.start as i32 + offset).max(0) as u32;
            let end = (right_span.end as i32 + offset).max(0) as u32;
            out.default_value_ranges.push(Range::new(start, end));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::{ScriptLang, parse_script_body};

    /// Parse `let <slice> = 0;`, walk the declarator's pattern, and
    /// hand the result to `f`. The closure form keeps the wrapped
    /// source alive for the parse's borrow lifetime — without it the
    /// returned `BindingPattern` would dangle.
    ///
    /// Offset is `-4` to match lint's `declare_each_context`: the
    /// `let ` prefix is 4 bytes and the pattern slice would be at
    /// byte 0 in the notional original source.
    fn with_pattern_bindings<R>(slice: &str, f: impl FnOnce(PatternBindings) -> R) -> R {
        let wrapped = format!("let {slice} = 0;");
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, &wrapped, ScriptLang::Ts);
        let stmt = parsed
            .program
            .body
            .first()
            .expect("expected at least one statement");
        let oxc_ast::ast::Statement::VariableDeclaration(vd) = stmt else {
            panic!("expected VariableDeclaration");
        };
        let d = vd.declarations.first().expect("one declarator");
        let pb = collect_pattern_bindings(&d.id, -4);
        f(pb)
    }

    fn names(pb: &PatternBindings) -> Vec<&str> {
        pb.bindings.iter().map(|b| b.name.as_str()).collect()
    }

    #[test]
    fn simple_identifier_emits_one_binding() {
        with_pattern_bindings("x", |pb| {
            assert_eq!(names(&pb), vec!["x"]);
            assert!(!pb.bindings[0].inside_rest);
            assert!(pb.default_value_ranges.is_empty());
        });
    }

    #[test]
    fn object_destructure_emits_each_property() {
        with_pattern_bindings("{ a, b, c }", |pb| {
            assert_eq!(names(&pb), vec!["a", "b", "c"]);
        });
    }

    #[test]
    fn object_rest_marks_rest_binding() {
        with_pattern_bindings("{ a, ...rest }", |pb| {
            assert_eq!(names(&pb), vec!["a", "rest"]);
            assert!(!pb.bindings[0].inside_rest);
            assert!(pb.bindings[1].inside_rest);
        });
    }

    #[test]
    fn array_rest_emits_tail_with_inside_rest_flag() {
        with_pattern_bindings("[head, ...tail]", |pb| {
            assert_eq!(names(&pb), vec!["head", "tail"]);
            assert!(!pb.bindings[0].inside_rest);
            assert!(pb.bindings[1].inside_rest);
        });
    }

    #[test]
    fn nested_object_inside_array_recurses() {
        with_pattern_bindings("[{ a, b }, c]", |pb| {
            assert_eq!(names(&pb), vec!["a", "b", "c"]);
        });
    }

    #[test]
    fn assignment_pattern_emits_binding_and_default_range() {
        with_pattern_bindings("{ a = 1 }", |pb| {
            assert_eq!(names(&pb), vec!["a"]);
            assert_eq!(pb.default_value_ranges.len(), 1);
        });
    }

    #[test]
    fn multiple_defaults_in_one_pattern_emit_in_order() {
        with_pattern_bindings("{ a = 1, b = 2, c }", |pb| {
            assert_eq!(names(&pb), vec!["a", "b", "c"]);
            assert_eq!(pb.default_value_ranges.len(), 2);
            assert!(pb.default_value_ranges[0].start < pb.default_value_ranges[1].start);
        });
    }

    #[test]
    fn ranges_translated_through_offset() {
        // Manual offset test — `let foo = 0;` puts `foo` at bytes
        // 4..7. With offset = 96, expect range 100..103 (mimics lint's
        // pattern slice starting at byte 100 in the original source).
        let wrapped = "let foo = 0;";
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, wrapped, ScriptLang::Ts);
        let oxc_ast::ast::Statement::VariableDeclaration(vd) = parsed.program.body.first().unwrap()
        else {
            panic!()
        };
        let d = vd.declarations.first().unwrap();
        let pb = collect_pattern_bindings(&d.id, 96);
        assert_eq!(pb.bindings[0].range, Range::new(100, 103));
    }

    #[test]
    fn rest_inside_nested_object_carries_flag() {
        // `{ outer: { inner, ...inner_rest } }` — `inner_rest` should
        // get inside_rest=true; `inner` should not.
        with_pattern_bindings("{ outer: { inner, ...inner_rest } }", |pb| {
            assert_eq!(names(&pb), vec!["inner", "inner_rest"]);
            assert!(!pb.bindings[0].inside_rest);
            assert!(pb.bindings[1].inside_rest);
        });
    }
}
