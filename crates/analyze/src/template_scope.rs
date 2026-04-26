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
//! Phase 1 — pattern primitive:
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
//! Phase 3 — scope-tracking walker:
//!
//! - [`ScopeKind`] — discriminator for each scope-introducing block.
//! - [`TemplateScopeVisitor`] — trait consumers implement to receive
//!   `enter_scope` / `leave_scope` / per-node `visit_*` calls.
//! - [`walk_with_visitor`] — drives recursion through the template
//!   AST and brackets every scope construct.
//! - [`extract_at_const_name`] — pulls the binding name out of a
//!   `{@const NAME = EXPR}` interpolation body.
//!
//! ### What does NOT live here yet
//!
//! Phase 4 (the lint migration) brings `svn-lint::scope::TreeBuilder`
//! onto `walk_with_visitor` via a `LintScopeVisitor`. Until then lint
//! still drives its own template walker.

use oxc_ast::ast::{BindingPattern, BindingPatternKind};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{Component, Element, Fragment, Node, SvelteElement};

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

// =====================================================================
// Phase 3: scope-tracking walker.
//
// `walk_with_visitor` drives the recursion through Fragment/Node and
// hands per-node + per-scope work to a `TemplateScopeVisitor` impl.
// Today only `analyze::template_walker::AnalyzeVisitor` consumes it;
// Phase 4 of `notes/PLAN-template-scope-unification.md` adds a
// `LintScopeVisitor` for `svn-lint::scope`'s `TreeBuilder`.
// =====================================================================

/// What kind of scope a `enter_scope` / `leave_scope` pair is
/// bracketing. Drives the visitor's per-kind binding-tagging
/// (lint maps Each/Snippet/LetDirective/AtConstScope to different
/// `BindingKind`s; analyze ignores the discriminator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// `walk_fragment` entry/exit. Used to bracket fragment-scoped
    /// constructs whose own scope has no per-tag closer (`{@const}`).
    Fragment,
    /// `{#each X as PAT}` body.
    Each,
    /// `{:then PAT}` branch of `{#await}`.
    AwaitThen,
    /// `{:catch PAT}` branch of `{#await}`.
    AwaitCatch,
    /// `{#snippet name(PAT)}` body.
    Snippet,
    /// `<Comp let:foo>` / `<el let:foo>` — let-directive bindings on
    /// an element/component scope its children.
    LetDirective,
}

/// Visitor invoked by [`walk_with_visitor`] at every scope boundary
/// and per-node site. Default impls are no-ops so consumers only
/// implement the methods they care about.
#[allow(unused_variables)]
pub trait TemplateScopeVisitor {
    /// Called at fragment entry. The visitor brackets its scope-stack
    /// here so any `visit_at_const` push during the walk is truncated
    /// at fragment exit.
    fn enter_fragment(&mut self) {}
    fn leave_fragment(&mut self) {}

    /// Called when entering a scope-introducing block. `bindings`
    /// carries the identifiers the construct declares (per-pattern,
    /// in walk order).
    fn enter_scope(&mut self, kind: ScopeKind, bindings: &[BoundIdent]) {}
    fn leave_scope(&mut self, kind: ScopeKind) {}

    /// Visited at element entry, BEFORE recursion into children. The
    /// visitor handles attribute-level work (`bind:`, `on:`, slot
    /// captures, action attrs) here. The walker handles let-directive
    /// scope-bracketing around the children.
    fn visit_element(&mut self, element: &Element) {}

    /// Visited at component entry, BEFORE recursion into children.
    /// Same role as `visit_element` but for `<Foo …>` instead of
    /// `<div …>`.
    fn visit_component(&mut self, component: &Component) {}

    /// Visited at `<svelte:element>` entry, BEFORE recursion. The
    /// dynamic-tag form behaves like an element for scope purposes.
    fn visit_svelte_element(&mut self, element: &SvelteElement) {}

    /// Visited at each-block entry, BEFORE the each-scope is entered.
    /// Analyze increments `summary.each_block_count` here.
    fn visit_each_block(&mut self, block: &svn_parser::EachBlock) {}

    /// `{@const NAME = EXPR}` interpolation. The walker has already
    /// extracted the binding name; the visitor records the name in
    /// its summary AND pushes onto its scope stack so subsequent
    /// sibling sites in the same fragment see it. (No matching
    /// leave-call — fragment-level bracket handles cleanup.)
    fn visit_at_const(&mut self, name: SmolStr, expr_range: Range) {}
}

/// Drive the visitor over a template fragment. Handles all
/// scope-introducing constructs (`{#each}`, `{#snippet}`, await
/// then/catch, `<… let:foo>`) by parsing the binding pattern,
/// calling `enter_scope(kind, bindings)`, recursing children, then
/// `leave_scope(kind)`. The visitor is responsible for all per-node
/// domain work via `visit_element` / `visit_component` /
/// `visit_svelte_element` / `visit_at_const`.
pub fn walk_with_visitor<V: TemplateScopeVisitor>(
    fragment: &Fragment,
    source: &str,
    visitor: &mut V,
) {
    walk_fragment_inner(fragment, source, visitor);
}

fn walk_fragment_inner<V: TemplateScopeVisitor>(
    fragment: &Fragment,
    source: &str,
    visitor: &mut V,
) {
    visitor.enter_fragment();
    for node in &fragment.nodes {
        walk_node_inner(node, source, visitor);
    }
    visitor.leave_fragment();
}

fn walk_node_inner<V: TemplateScopeVisitor>(node: &Node, source: &str, visitor: &mut V) {
    match node {
        Node::Element(e) => {
            visitor.visit_element(e);
            let let_bindings = collect_let_directive_bindings(&e.attributes, source);
            visitor.enter_scope(ScopeKind::LetDirective, &let_bindings);
            walk_fragment_inner(&e.children, source, visitor);
            visitor.leave_scope(ScopeKind::LetDirective);
        }
        Node::Component(c) => {
            visitor.visit_component(c);
            let let_bindings = collect_let_directive_bindings(&c.attributes, source);
            visitor.enter_scope(ScopeKind::LetDirective, &let_bindings);
            walk_fragment_inner(&c.children, source, visitor);
            visitor.leave_scope(ScopeKind::LetDirective);
        }
        Node::SvelteElement(s) => {
            visitor.visit_svelte_element(s);
            let let_bindings = collect_let_directive_bindings(&s.attributes, source);
            visitor.enter_scope(ScopeKind::LetDirective, &let_bindings);
            walk_fragment_inner(&s.children, source, visitor);
            visitor.leave_scope(ScopeKind::LetDirective);
        }
        Node::IfBlock(b) => {
            walk_fragment_inner(&b.consequent, source, visitor);
            for arm in &b.elseif_arms {
                walk_fragment_inner(&arm.body, source, visitor);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment_inner(alt, source, visitor);
            }
        }
        Node::EachBlock(b) => {
            visitor.visit_each_block(b);
            // `{#each X as item, i (key)}`. `item` and `i` enter scope
            // for the body. The pattern in `context_range` may be a
            // destructure (`as { a, b }`); the index is always a bare
            // identifier.
            let mut bindings: Vec<BoundIdent> = Vec::new();
            if let Some(as_clause) = &b.as_clause {
                bindings.extend(collect_pattern_bindings_from_slice(
                    source,
                    as_clause.context_range,
                ));
                if let Some(idx) = &as_clause.index_range {
                    bindings.extend(collect_pattern_bindings_from_slice(source, *idx));
                }
            }
            visitor.enter_scope(ScopeKind::Each, &bindings);
            walk_fragment_inner(&b.body, source, visitor);
            visitor.leave_scope(ScopeKind::Each);
            if let Some(alt) = &b.alternate {
                // `{:else}` branch — empty-list body. No bindings in
                // scope (the `as` binding doesn't apply here).
                walk_fragment_inner(alt, source, visitor);
            }
        }
        Node::AwaitBlock(b) => {
            if let Some(p) = &b.pending {
                walk_fragment_inner(p, source, visitor);
            }
            if let Some(t) = &b.then_branch {
                let bindings = match &t.context_range {
                    Some(r) => collect_pattern_bindings_from_slice(source, *r),
                    None => Vec::new(),
                };
                visitor.enter_scope(ScopeKind::AwaitThen, &bindings);
                walk_fragment_inner(&t.body, source, visitor);
                visitor.leave_scope(ScopeKind::AwaitThen);
            }
            if let Some(c) = &b.catch_branch {
                let bindings = match &c.context_range {
                    Some(r) => collect_pattern_bindings_from_slice(source, *r),
                    None => Vec::new(),
                };
                visitor.enter_scope(ScopeKind::AwaitCatch, &bindings);
                walk_fragment_inner(&c.body, source, visitor);
                visitor.leave_scope(ScopeKind::AwaitCatch);
            }
        }
        Node::KeyBlock(b) => walk_fragment_inner(&b.body, source, visitor),
        Node::SnippetBlock(b) => {
            let bindings = collect_pattern_bindings_from_slice(source, b.parameters_range);
            visitor.enter_scope(ScopeKind::Snippet, &bindings);
            walk_fragment_inner(&b.body, source, visitor);
            visitor.leave_scope(ScopeKind::Snippet);
        }
        Node::Interpolation(i) if i.kind == svn_parser::InterpolationKind::AtConst => {
            if let Some(name) = extract_at_const_name(i, source) {
                visitor.visit_at_const(name, i.expression_range);
            }
        }
        // Leaf nodes — no children, no scope, nothing for the visitor.
        Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
    }
}

/// Parse a binding-pattern source slice (the `as` clause of
/// `{#each}`, an await branch's `{value}`, or a snippet's
/// `(p1, p2)` list) and return the bindings it declares.
///
/// Used by the walker to feed `enter_scope`. The slice form means
/// the parser-wrapper picks `(slice) => 0` (works for both bare
/// identifiers and destructures) — matches what
/// `analyze::template_walker::collect_pattern_idents` was doing
/// before Phase 1's primitive extraction.
fn collect_pattern_bindings_from_slice(source: &str, range: Range) -> Vec<BoundIdent> {
    let start = range.start as usize;
    let end = range.end as usize;
    let Some(slice) = source.get(start..end) else {
        return Vec::new();
    };
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }
    let alloc = oxc_allocator::Allocator::default();
    let wrapped = format!("({trimmed}) => 0");
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return Vec::new();
    }
    use oxc_ast::ast::{Expression, Statement};
    let Some(stmt) = parsed.program.body.first() else {
        return Vec::new();
    };
    let Statement::ExpressionStatement(es) = stmt else {
        return Vec::new();
    };
    let Expression::ArrowFunctionExpression(arrow) = &es.expression else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for param in &arrow.params.items {
        // Offset is irrelevant here — the walker uses bindings only
        // to feed `enter_scope`; the visitor's scope-stack stores
        // names, not source ranges. (Lint's Phase-4 visitor will
        // need real ranges; that's tracked there.)
        let pb = collect_pattern_bindings(&param.pattern, 0);
        out.extend(pb.bindings);
    }
    out
}

/// Extract every `let:foo` / `let:foo={pattern}` binding from an
/// element's attribute list. Each binding becomes a `BoundIdent` so
/// `enter_scope(LetDirective, …)` can feed the same shape as
/// each-block / snippet pattern bindings.
///
/// Shorthand `let:foo` (no `={…}`) declares the directive name
/// itself; expression form `let:foo={x}` uses the expression slice
/// — usually a bare identifier (`x`) but may be a destructure
/// (`{a, b}` or `[x, y]`).
fn collect_let_directive_bindings(
    attrs: &[svn_parser::Attribute],
    source: &str,
) -> Vec<BoundIdent> {
    use svn_parser::{Attribute, Directive, DirectiveKind, DirectiveValue};
    let mut out: Vec<BoundIdent> = Vec::new();
    let mut seen: Vec<SmolStr> = Vec::new();
    let push = |b: BoundIdent, out: &mut Vec<BoundIdent>, seen: &mut Vec<SmolStr>| {
        if !seen.iter().any(|n| n == &b.name) {
            seen.push(b.name.clone());
            out.push(b);
        }
    };
    for attr in attrs {
        let Attribute::Directive(Directive {
            kind: DirectiveKind::Let,
            name,
            value,
            ..
        }) = attr
        else {
            continue;
        };
        match value {
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                let bindings = collect_pattern_bindings_from_slice(source, *expression_range);
                if bindings.is_empty() {
                    // Empty expression — fall back to the directive name itself.
                    push(
                        BoundIdent {
                            name: name.clone(),
                            range: Range::new(0, 0),
                            inside_rest: false,
                        },
                        &mut out,
                        &mut seen,
                    );
                } else {
                    for b in bindings {
                        push(b, &mut out, &mut seen);
                    }
                }
            }
            // Shorthand: `let:foo` declares `foo`.
            _ => push(
                BoundIdent {
                    name: name.clone(),
                    range: Range::new(0, 0),
                    inside_rest: false,
                },
                &mut out,
                &mut seen,
            ),
        }
    }
    out
}

/// Pull the binding name out of a `{@const NAME = EXPR}` interpolation
/// body. Returns `None` for destructure patterns (body starts with `{`)
/// or malformed input — both match upstream's behaviour of emitting
/// nothing for those cases.
pub fn extract_at_const_name(interp: &svn_parser::Interpolation, source: &str) -> Option<SmolStr> {
    let start = interp.expression_range.start as usize;
    let end = interp.expression_range.end as usize;
    let body = source.get(start..end)?;
    let bytes = body.as_bytes();
    let mut p = 0usize;
    while p < bytes.len()
        && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_' || bytes[p] == b'$')
    {
        p += 1;
    }
    if p == 0 {
        return None;
    }
    Some(SmolStr::from(&body[..p]))
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
