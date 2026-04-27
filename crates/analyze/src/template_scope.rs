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
//! - [`extract_at_const_bindings`] — pulls every identifier introduced
//!   by a `{@const NAME = EXPR}` (or destructure form) interpolation body.
//!
//! ### Consumers
//!
//! - `analyze::template_walker::AnalyzeVisitor` — populates
//!   `TemplateSummary` (component instantiations, slot-defs, action
//!   attrs, bind-this, `{@const}` shadow).
//! - `svn-lint::scope::LintScopeVisitor` (Phase 4) — drives
//!   `TreeBuilder`'s declarations + reference-recording + control-
//!   flow tracking.

use oxc_ast::ast::BindingPattern;
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
    match pat {
        BindingPattern::BindingIdentifier(id) => {
            let start = (id.span.start as i32 + offset).max(0) as u32;
            let end = (id.span.end as i32 + offset).max(0) as u32;
            out.bindings.push(BoundIdent {
                name: SmolStr::from(id.name.as_str()),
                range: Range::new(start, end),
                inside_rest,
            });
        }
        BindingPattern::ObjectPattern(op) => {
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
        BindingPattern::ArrayPattern(ap) => {
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
        BindingPattern::AssignmentPattern(asn) => {
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
// Both `analyze::template_walker::AnalyzeVisitor` and
// `svn-lint::scope::LintScopeVisitor` (Phase 4) consume it.
// =====================================================================

/// What kind of scope a `enter_scope` / `leave_scope` pair is
/// bracketing. Drives the visitor's per-kind binding-tagging
/// (lint maps Each/Snippet/LetDirective to different `BindingKind`s;
/// analyze ignores the discriminator).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScopeKind {
    /// `walk_fragment` entry/exit. Used to bracket fragment-scoped
    /// constructs whose own scope has no per-tag closer (`{@const}`).
    Fragment,
    /// `{#each X as PAT [, INDEX] [(KEY)]}` body. Carries the
    /// `is_keyed` flag (for the index binding's `Static` vs
    /// `Template` kind in lint) and `has_index` (which tells the
    /// visitor whether the LAST entry in the bindings array passed
    /// to `enter_scope` is the index).
    Each { is_keyed: bool, has_index: bool },
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

    /// Visited at if-block entry, BEFORE control-flow / expression
    /// walks. Lint walks branches' condition expressions at the
    /// walker-emitted `visit_expr` calls; this hook is reserved for
    /// future per-block work.
    fn visit_if_block(&mut self, block: &svn_parser::IfBlock) {}

    /// Visited at await-block entry, BEFORE control-flow / expression
    /// walks.
    fn visit_await_block(&mut self, block: &svn_parser::AwaitBlock) {}

    /// Visited at key-block entry, BEFORE control-flow / expression
    /// walks.
    fn visit_key_block(&mut self, block: &svn_parser::KeyBlock) {}

    /// Visited at snippet-block entry, BEFORE the snippet scope is
    /// entered.
    fn visit_snippet_block(&mut self, block: &svn_parser::SnippetBlock) {}

    /// Walk an expression at `range` in the CURRENT scope context
    /// (the visitor decides which scope from its own state — for
    /// lint, that's `ctx.scope`). Default no-op; lint implements via
    /// `walk_expr_range`. Walker calls this for: `{#if}` and elseif
    /// condition ranges, `{#each}` outer expression + key, `{#await}`
    /// expression, `{#key}` expression, and any `{EXPR}`
    /// interpolation that's not `{@const …}`.
    fn visit_expr(&mut self, range: Range) {}

    /// Bracket entry into a control-flow block (`{#if}` / `{#each}` /
    /// `{#await}` / `{#key}`). Lint's `TemplateCtx.in_control_flow`
    /// flips `true` between these calls; analyze ignores them.
    fn enter_control_flow(&mut self) {}
    fn leave_control_flow(&mut self) {}

    /// `{@const NAME = EXPR}` interpolation. Walker passes the full
    /// expression range (the body slice between `{@const ` and `}`)
    /// for visitors that re-parse the body themselves (lint re-parses
    /// as `let X = EXPR;` to handle destructure patterns + initialiser
    /// walking). `bound_names` is the full identifier list extracted
    /// from the pattern — bare-identifier form yields one name;
    /// destructure form (`{@const { a, b } = X}`) yields all
    /// identifiers in walk order. Analyze's shadow tracking pushes
    /// every name; lint ignores the list and re-parses for
    /// declarations.
    fn visit_at_const(&mut self, bound_names: &[SmolStr], expr_range: Range) {}
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
            walk_element_children(&e.attributes, &e.children, source, visitor);
        }
        Node::Component(c) => {
            visitor.visit_component(c);
            walk_element_children(&c.attributes, &c.children, source, visitor);
        }
        Node::SvelteElement(s) => {
            visitor.visit_svelte_element(s);
            walk_element_children(&s.attributes, &s.children, source, visitor);
        }
        Node::IfBlock(b) => {
            visitor.visit_if_block(b);
            // Outer condition walks in the parent's `in_control_flow`
            // (it's the discriminator deciding control flow, not yet
            // inside it). Mirrors existing lint walker:
            //   walk_expr(condition_range)   <- parent control-flow
            //   in_control_flow = true
            //   walk consequent / arms / alternate
            //   in_control_flow = saved
            visitor.visit_expr(b.condition_range);
            visitor.enter_control_flow();
            walk_fragment_inner(&b.consequent, source, visitor);
            for arm in &b.elseif_arms {
                // Elseif conditions walk INSIDE the in_control_flow
                // bracket (matches existing lint behaviour).
                visitor.visit_expr(arm.condition_range);
                walk_fragment_inner(&arm.body, source, visitor);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment_inner(alt, source, visitor);
            }
            visitor.leave_control_flow();
        }
        Node::EachBlock(b) => {
            visitor.visit_each_block(b);
            // Outer expression walks in the PARENT scope, in the
            // parent's `in_control_flow` (it's the iterable, outside
            // the each-block's body).
            visitor.visit_expr(b.expression_range);
            // `{#each X as item, i (key)}`. `item` and `i` enter
            // scope for the body. Pattern in `context_range` may be a
            // destructure; index is always a bare identifier.
            // Bindings convention: context entries first, index last
            // when present (consumers gate on `has_index`).
            let mut bindings: Vec<BoundIdent> = Vec::new();
            let mut defaults: Vec<Range> = Vec::new();
            let mut has_index = false;
            let mut is_keyed = false;
            let mut key_range: Option<Range> = None;
            if let Some(as_clause) = &b.as_clause {
                let context = collect_pattern_bindings_from_slice(source, as_clause.context_range);
                bindings.extend(context.bindings);
                defaults.extend(context.default_value_ranges);
                if let Some(idx) = &as_clause.index_range {
                    let i = collect_pattern_bindings_from_slice(source, *idx);
                    bindings.extend(i.bindings);
                    defaults.extend(i.default_value_ranges);
                    has_index = true;
                }
                if let Some(k) = as_clause.key_range {
                    key_range = Some(k);
                    is_keyed = true;
                }
            }
            // Default-value expressions walk in the PARENT scope,
            // BEFORE entering the each-block's child scope — matches
            // upstream's `{ a = b }` semantics where `b` resolves to
            // a parent binding, not the just-declared `a`.
            for default in &defaults {
                visitor.visit_expr(*default);
            }
            visitor.enter_scope(
                ScopeKind::Each {
                    is_keyed,
                    has_index,
                },
                &bindings,
            );
            // Key expression walks in the CHILD scope (key may
            // reference the each binding) but in the PARENT's
            // `in_control_flow` — same as the outer expression.
            if let Some(k) = key_range {
                visitor.visit_expr(k);
            }
            // Body recursion is the first site that sees
            // `in_control_flow = true`.
            visitor.enter_control_flow();
            walk_fragment_inner(&b.body, source, visitor);
            visitor.leave_scope(ScopeKind::Each {
                is_keyed,
                has_index,
            });
            if let Some(alt) = &b.alternate {
                // `{:else}` walks in the parent scope but stays
                // INSIDE the `in_control_flow` bracket — matches
                // existing lint behaviour where `saved` restores
                // only after the alternate.
                walk_fragment_inner(alt, source, visitor);
            }
            visitor.leave_control_flow();
        }
        Node::AwaitBlock(b) => {
            visitor.visit_await_block(b);
            // Awaited expression walks in the parent's
            // `in_control_flow` (the promise is resolved outside
            // the body).
            visitor.visit_expr(b.expression_range);
            visitor.enter_control_flow();
            if let Some(p) = &b.pending {
                walk_fragment_inner(p, source, visitor);
            }
            if let Some(t) = &b.then_branch {
                let pb = match &t.context_range {
                    Some(r) => collect_pattern_bindings_from_slice(source, *r),
                    None => PatternBindings::default(),
                };
                for default in &pb.default_value_ranges {
                    visitor.visit_expr(*default);
                }
                visitor.enter_scope(ScopeKind::AwaitThen, &pb.bindings);
                walk_fragment_inner(&t.body, source, visitor);
                visitor.leave_scope(ScopeKind::AwaitThen);
            }
            if let Some(c) = &b.catch_branch {
                let pb = match &c.context_range {
                    Some(r) => collect_pattern_bindings_from_slice(source, *r),
                    None => PatternBindings::default(),
                };
                for default in &pb.default_value_ranges {
                    visitor.visit_expr(*default);
                }
                visitor.enter_scope(ScopeKind::AwaitCatch, &pb.bindings);
                walk_fragment_inner(&c.body, source, visitor);
                visitor.leave_scope(ScopeKind::AwaitCatch);
            }
            visitor.leave_control_flow();
        }
        Node::KeyBlock(b) => {
            visitor.visit_key_block(b);
            // Key expression walks in the parent's `in_control_flow`.
            visitor.visit_expr(b.expression_range);
            visitor.enter_control_flow();
            walk_fragment_inner(&b.body, source, visitor);
            visitor.leave_control_flow();
        }
        Node::SnippetBlock(b) => {
            visitor.visit_snippet_block(b);
            let pb = if b.parameters_range.start < b.parameters_range.end {
                collect_pattern_bindings_from_slice(source, b.parameters_range)
            } else {
                PatternBindings::default()
            };
            for default in &pb.default_value_ranges {
                visitor.visit_expr(*default);
            }
            visitor.enter_scope(ScopeKind::Snippet, &pb.bindings);
            walk_fragment_inner(&b.body, source, visitor);
            visitor.leave_scope(ScopeKind::Snippet);
        }
        Node::Interpolation(i) if i.kind == svn_parser::InterpolationKind::AtConst => {
            // Walker emits the FULL bound-names list (handles both
            // bare-identifier and destructure forms — see
            // `extract_at_const_bindings`) AND the full expression
            // range (used by lint to re-parse for initialiser
            // walking and binding declarations).
            let names = extract_at_const_bindings(i, source);
            visitor.visit_at_const(&names, i.expression_range);
        }
        Node::Interpolation(i) => {
            // Plain `{EXPR}` — walk the body as an expression in the
            // current scope.
            visitor.visit_expr(i.expression_range);
        }
        // Leaf nodes — no children, no scope, nothing for the visitor.
        Node::Text(_) | Node::Comment(_) => {}
    }
}

/// Drive an element-like node's body: visitor handles attributes
/// inside its `visit_element` / `visit_component` /
/// `visit_svelte_element` (called by the caller before this); walker
/// brackets the let-directive scope around the children.
///
/// Skips `enter_scope` entirely when the element has no let-directives
/// — preserves byte-equivalent behaviour against the pre-Phase-3 lint
/// walker, which only created a child scope when `has_let` was true.
fn walk_element_children<V: TemplateScopeVisitor>(
    attrs: &[svn_parser::Attribute],
    children: &Fragment,
    source: &str,
    visitor: &mut V,
) {
    let LetDirectiveScope {
        bindings: let_bindings,
        defaults: let_defaults,
    } = collect_let_directive_bindings(attrs, source);
    // Default-value expressions inside let-directive patterns
    // (`<Comp let:item={{ x = outer }}>`) walk in the PARENT scope
    // BEFORE entering the let-directive child scope — refs like
    // `outer` resolve to a parent binding, not the just-declared
    // `x`. Mirrors each-block / snippet-param default-value
    // handling in `walk_node_inner`.
    for default in &let_defaults {
        visitor.visit_expr(*default);
    }
    if let_bindings.is_empty() {
        walk_fragment_inner(children, source, visitor);
    } else {
        visitor.enter_scope(ScopeKind::LetDirective, &let_bindings);
        walk_fragment_inner(children, source, visitor);
        visitor.leave_scope(ScopeKind::LetDirective);
    }
}

/// Output of [`collect_let_directive_bindings`]: bindings to declare
/// in the let-directive child scope, plus default-value expression
/// ranges that the caller walks in PARENT scope before entering the
/// child scope.
struct LetDirectiveScope {
    bindings: Vec<BoundIdent>,
    defaults: Vec<Range>,
}

/// Parse a binding-pattern source slice (the `as` clause of
/// `{#each}`, an await branch's `{value}`, or a snippet's
/// `(p1, p2)` list) and return the bindings it declares plus any
/// default-value expression ranges, all in ORIGINAL source
/// coordinates.
///
/// Default-value ranges (e.g. the `expr` in `{ a = expr }`) are
/// emitted alongside the bindings so the caller — typically the
/// walker — can hand them to `visit_expr` in the PARENT scope
/// before entering the each/snippet/let-directive child scope. That
/// matches what the pre-Phase-4 lint walker did inline inside
/// `declare_each_pattern`'s `AssignmentPattern` arm.
///
/// Wrapper: `({trimmed}) => 0`. The `(` adds 1 byte of prefix in
/// the wrapped string, so the offset that translates oxc spans
/// back to original source = `range.start + trim_offset - 1`,
/// where `trim_offset` is the byte distance between `slice.start`
/// and `trimmed.start` (handles leading whitespace inside the
/// pattern slice).
fn collect_pattern_bindings_from_slice(source: &str, range: Range) -> PatternBindings {
    let start = range.start as usize;
    let end = range.end as usize;
    let Some(slice) = source.get(start..end) else {
        return PatternBindings::default();
    };
    let trimmed = slice.trim_start();
    let trim_byte_offset = slice.len() - trimmed.len();
    let trimmed = trimmed.trim_end();
    if trimmed.is_empty() {
        return PatternBindings::default();
    }
    let alloc = oxc_allocator::Allocator::default();
    let wrapped = format!("({trimmed}) => 0");
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return PatternBindings::default();
    }
    use oxc_ast::ast::{Expression, Statement};
    let Some(stmt) = parsed.program.body.first() else {
        return PatternBindings::default();
    };
    let Statement::ExpressionStatement(es) = stmt else {
        return PatternBindings::default();
    };
    let Expression::ArrowFunctionExpression(arrow) = &es.expression else {
        return PatternBindings::default();
    };
    // Wrapper-relative oxc span at byte N → original source byte
    // `(range.start + trim_byte_offset) + (N - 1)` (1 = the `(` prefix).
    let offset = range.start as i32 + trim_byte_offset as i32 - 1;
    let mut out = PatternBindings::default();
    for param in &arrow.params.items {
        let pb = collect_pattern_bindings(&param.pattern, offset);
        out.bindings.extend(pb.bindings);
        out.default_value_ranges.extend(pb.default_value_ranges);
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
) -> LetDirectiveScope {
    use svn_parser::{Attribute, Directive, DirectiveKind, DirectiveValue};
    let mut out = LetDirectiveScope {
        bindings: Vec::new(),
        defaults: Vec::new(),
    };
    let mut seen: Vec<SmolStr> = Vec::new();
    let push = |b: BoundIdent, out: &mut LetDirectiveScope, seen: &mut Vec<SmolStr>| {
        if !seen.iter().any(|n| n == &b.name) {
            seen.push(b.name.clone());
            out.bindings.push(b);
        }
    };
    for attr in attrs {
        let Attribute::Directive(Directive {
            kind: DirectiveKind::Let,
            name,
            value,
            range,
            ..
        }) = attr
        else {
            continue;
        };
        match value {
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                let pb = collect_pattern_bindings_from_slice(source, *expression_range);
                if pb.bindings.is_empty() {
                    // Empty expression — fall back to the directive name itself.
                    push(
                        BoundIdent {
                            name: name.clone(),
                            range: *range,
                            inside_rest: false,
                        },
                        &mut out,
                        &mut seen,
                    );
                } else {
                    for b in pb.bindings {
                        push(b, &mut out, &mut seen);
                    }
                    // Default-value expressions inside let-directive
                    // destructure patterns surface here so the walker
                    // can hand them to `visit_expr` in the PARENT
                    // scope before entering the let-directive child
                    // scope — see `walk_element_children`.
                    out.defaults.extend(pb.default_value_ranges);
                }
            }
            // Shorthand: `let:foo` declares `foo`.
            _ => push(
                BoundIdent {
                    name: name.clone(),
                    range: *range,
                    inside_rest: false,
                },
                &mut out,
                &mut seen,
            ),
        }
    }
    out
}

/// Pull every identifier introduced by a `{@const NAME = EXPR}` (or
/// `{@const { a, b } = EXPR}` / `{@const [x, ...rest] = EXPR}`)
/// interpolation body. Returns the names in walk order.
///
/// Fast path: bare-identifier form (`NAME = EXPR`) skips the oxc
/// parser. Destructure forms re-parse the body as
/// `let <body>;` and run the unified pattern walker so analyze's
/// shadow tracking picks them up — without this, a slot-attr after
/// `{@const { a } = X}` would resolve `a` against the wrong outer
/// binding.
pub fn extract_at_const_bindings(interp: &svn_parser::Interpolation, source: &str) -> Vec<SmolStr> {
    let start = interp.expression_range.start as usize;
    let end = interp.expression_range.end as usize;
    let Some(body) = source.get(start..end) else {
        return Vec::new();
    };
    // Bare identifier fast path: leading run of identifier chars.
    let bytes = body.as_bytes();
    let mut p = 0usize;
    while p < bytes.len()
        && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_' || bytes[p] == b'$')
    {
        p += 1;
    }
    if p > 0 {
        return vec![SmolStr::from(&body[..p])];
    }
    // Destructure form: re-parse as `let <body>;` and walk the
    // declarator's pattern. Wrapper prefix `let ` is 4 bytes; offset
    // is irrelevant here (analyze's consumer only reads names, not
    // ranges).
    let wrapped = format!("let {body};");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return Vec::new();
    }
    use oxc_ast::ast::Statement;
    let Some(Statement::VariableDeclaration(vd)) = parsed.program.body.first() else {
        return Vec::new();
    };
    let Some(d) = vd.declarations.first() else {
        return Vec::new();
    };
    let pb = collect_pattern_bindings(&d.id, 0);
    pb.bindings.into_iter().map(|b| b.name).collect()
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
