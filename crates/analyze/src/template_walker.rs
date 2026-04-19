//! Template walker — populates the SemanticModel from a parsed Fragment.
//!
//! Single AST walk that visits every node and dispatches to the relevant
//! collectors:
//!
//! - `use:` directives → register `__svn_action_attrs_N` in
//!   [`VoidRefRegistry`] (one per directive, counter shared workspace-wide
//!   per component).
//! - `bind:foo={getter, setter}` → register `__svn_bind_pair_N`.
//! - `bind:this={x}` where `x` is a simple identifier → record `x` as a
//!   bind-target. Emit later rewrites the matching `let x: T;` declaration
//!   in the script to `let x!: T;` so TypeScript's definite-assignment
//!   analysis doesn't flag closure reads (Svelte assigns asynchronously).
//! - Each block — counted; emit needs the count to generate unique loop
//!   binding names.
//!
//! This should ideally fuse with rune detection in a single visitor.
//! For now rune detection runs over the script AST (oxc) while template
//! walking is structural — different inputs, two passes. When we add a
//! `Visit` trait that bridges both, we'll fuse.

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{
    AttrValuePart, Attribute, Directive, DirectiveKind, DirectiveValue, Fragment, Node,
};

use crate::void_refs::VoidRefRegistry;

/// Per-template summary populated during the walk.
#[derive(Debug, Default, Clone)]
pub struct TemplateSummary {
    /// Names (registered upstream) that need void-references emitted.
    pub void_refs: VoidRefRegistry,
    /// `bind:this={x}` targets where `x` is a simple identifier — eligible
    /// for the definite-assignment rewrite.
    pub bind_this_targets: Vec<BindThisTarget>,
    /// DOM-element `bind:NAME={x}` directives on a narrow allowlist of
    /// one-way-not-on-element bindings (`bind:contentRect`,
    /// `bind:contentBoxSize`, etc. — see `dom_binding::type_for`).
    /// These don't live on the element's own type, so upstream emits
    /// `x = elem.NAME as <Type>` as an assertion. We emit a simpler
    /// shape — `x = __svn_any() as <Type>` — which checks that the
    /// binding target's declared type accepts the binding's value
    /// type (TS2322 fires on `let rect: string; bind:contentRect={rect}`
    /// because DOMRectReadOnly isn't assignable to string).
    pub dom_bindings: Vec<DomBinding>,
    /// Number of `{#each}` blocks encountered. Emit uses this to allocate
    /// unique iteration helpers.
    pub each_block_count: usize,
    /// Names introduced by `{@const NAME = expr}` template tags. These
    /// are template-scope locals that don't exist in the script. Emit
    /// declares each as `let NAME: any;` inside `__svn_tpl_check` so
    /// downstream `{#if NAME.x}` / `{#each NAME as ...}` references
    /// don't fire TS2304.
    pub at_const_names: Vec<SmolStr>,
    /// Each `<Component prop1=... prop2=... />` instantiation we found,
    /// with enough info for emit to generate a `satisfies
    /// ComponentProps<typeof Component>` type-check that catches
    /// excess-property errors on the user's prop list.
    ///
    /// Components with directives (`bind:`, `on:`, `use:`, `class:`,
    /// `style:`, transitions, animations) or spreads are excluded — for
    /// those, the satisfies object would be incomplete in a way that
    /// would itself cause false positives. Component-prop checking for
    /// those shapes is a future expansion.
    pub component_instantiations: Vec<ComponentInstantiation>,
}

/// One `<Component ...>` site eligible for excess-property checking.
#[derive(Debug, Clone)]
pub struct ComponentInstantiation {
    /// Root identifier of the component name (e.g. `MyButton` from
    /// `<MyButton />` or `<ui.MyButton />`).
    pub component_root: SmolStr,
    /// Plain attributes + translated `bind:NAME={x}` directives (as
    /// `Expression` props) + `{...expr}` spreads. Excludes
    /// `on:event` (tracked separately in `on_events`), `bind:this`,
    /// `use:`, `class:`, transitions.
    pub props: Vec<PropShape>,
    /// True when the user put NON-snippet, non-whitespace template
    /// content between the component's open/close tags. At runtime
    /// this becomes an implicit `children: Snippet` prop. Emit
    /// synthesizes `"children": () => __svn_snippet_return()` in
    /// the props literal when true so a component declaring
    /// `children: Snippet` (required) accepts `<Comp>body</Comp>`
    /// without firing a spurious TS2741 at phase 5's `satisfies`
    /// trailer.
    ///
    /// Pure `{#snippet}` children do NOT count — they hoist as
    /// their own explicit props via emit's snippet-as-arrow-prop
    /// path. Pure whitespace (indentation / newlines between open
    /// and close) also doesn't count.
    pub has_implicit_children: bool,
    /// Identifier name from `<Comp bind:this={x}>` when `x` is a
    /// simple identifier. Emit writes `x = $$_inst;` after
    /// construction to type-check `x`'s declared type against the
    /// component instance. Member-expression forms (`bind:this={refs.x}`)
    /// stay `None` for now — upstream handles those with a
    /// `(setter)(instance)` shape we haven't ported.
    pub bind_this_target: Option<SmolStr>,
    /// SVELTE-4-COMPAT: `on:event={handler}` directives on this
    /// component. Emit binds each via `$inst.$on("event", handler)`
    /// on the hoisted instance local, mirroring upstream svelte2tsx's
    /// shape. The props object stays free of `on*` keys so we can
    /// drop the `on${string}` union from `__SvnPropsPartial` — which
    /// in turn stops collisions with user props whose names start
    /// with "on" (`oneTouchReaction`, `onVideoMoments`, etc.).
    pub on_events: Vec<OnEventDirective>,
    /// Byte offset of the `<Component` token in the source. Emit keys
    /// the prop-check on this to locate the correct enclosing scope
    /// (i.e. inside the right `{#each}` / `{#if}` / `{#snippet}` body)
    /// when re-walking the template fragment.
    pub node_start: u32,
}

/// One `on:event={handler}` directive on a component instantiation.
/// Gets emitted as `$inst.$on("event", handler)` after construction.
#[derive(Debug, Clone)]
pub struct OnEventDirective {
    /// The event name without the `on:` prefix (e.g. `click` for
    /// `on:click`). Modifiers are stripped — runtime behavior, no
    /// type signature impact.
    pub event_name: SmolStr,
    /// Source range of the handler expression. Empty (start == end)
    /// when the user wrote `on:event` with no `={…}` value — those
    /// re-dispatch the event to a parent listener at runtime; we
    /// skip emit for type-check purposes.
    pub handler_range: Range,
}

/// One prop on a component instantiation.
#[derive(Debug, Clone)]
pub enum PropShape {
    /// `name="literal"` — quoted string value with no interpolation.
    Literal { name: SmolStr, value: String },
    /// `name={expr}` — emit the expression source verbatim as the value.
    Expression { name: SmolStr, expr_range: Range },
    /// `{name}` — shorthand `name={name}`.
    Shorthand { name: SmolStr },
    /// `name` (no `=`) — boolean shorthand.
    BoolShorthand { name: SmolStr },
    /// `{...expr}` spread — emit as `...(expr)` in the props literal.
    /// TS type-checks the spread's inferred shape against the
    /// destination type; mismatched spreads fire the usual structural
    /// mismatch errors. Unlike named props, spreads can contribute
    /// any subset of the declared Props AND extra keys in a single
    /// expression.
    Spread { expr_range: Range },
}

/// One `bind:this={x}` site.
#[derive(Debug, Clone)]
pub struct BindThisTarget {
    /// The identifier name `x`.
    pub name: SmolStr,
    /// Source range of the bind expression (the `x` part).
    pub range: Range,
}

/// One `bind:NAME={x}` site on a DOM element where NAME is in
/// the one-way-not-on-element allowlist (e.g. `contentRect`,
/// `contentBoxSize`). Emit uses this to generate the assignment
/// type-check `__svn_any_as<TYPE>(x)`.
#[derive(Debug, Clone)]
pub struct DomBinding {
    /// Either the user's `={expr}` range (when explicit) or a plain
    /// identifier for the bare-shorthand form `bind:NAME` (which
    /// desugars to `bind:NAME={NAME}`). Emit copies this into the
    /// phantom helper call's argument slot.
    pub expression: DomBindingExpression,
    /// TypeScript type the phantom helper's generic uses (e.g.
    /// `"DOMRectReadOnly"`). Comes from the per-binding table in
    /// `dom_binding::type_for`.
    pub type_annotation: &'static str,
}

#[derive(Debug, Clone)]
pub enum DomBindingExpression {
    /// Source range covering the user's `={expr}` value; emit reads
    /// the source slice verbatim.
    Range(Range),
    /// Bare shorthand `bind:NAME` desugars to `bind:NAME={NAME}` —
    /// the identifier to pass is the directive's own name.
    Identifier(SmolStr),
}

/// Walk the template fragment, collecting synthesized-name registrations
/// and bind-target metadata.
///
/// `source` is the original component source — needed to extract identifier
/// text from byte ranges (e.g. for `bind:this={x}`).
pub fn walk_template(fragment: &Fragment, source: &str) -> TemplateSummary {
    let mut summary = TemplateSummary::default();
    summary.void_refs.register("__svn_tpl_check");
    let mut counters = Counters::default();
    let mut ctx = WalkCtx { source };
    walk_fragment(fragment, &mut summary, &mut counters, &ctx);
    let _ = &mut ctx;
    collect_at_const_names(source, &mut summary.at_const_names);
    summary
}

/// Source-level scan for `{@const NAME = ...}` declarations.
///
/// The structural parser models `@const` as a generic interpolation
/// (we deliberately punt on per-tag semantics there), which loses the
/// LHS / RHS split. Re-scanning the raw source for the literal pattern
/// is the simplest path to the bound name without changing the AST
/// shape.
///
/// Multiple `{@const}` declarations and the same name across separate
/// templates are deduped so emit doesn't generate `let X: any;` twice
/// (TS2451 redeclaration).
fn collect_at_const_names(source: &str, out: &mut Vec<SmolStr>) {
    let bytes = source.as_bytes();
    let needle = b"{@const";
    let mut i = 0;
    let mut seen = std::collections::HashSet::<SmolStr>::new();
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        let mut p = i + needle.len();
        // Require whitespace after `@const`.
        if p >= bytes.len() || !bytes[p].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        let name_start = p;
        while p < bytes.len()
            && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_' || bytes[p] == b'$')
        {
            p += 1;
        }
        if p == name_start {
            i += 1;
            continue;
        }
        let name = SmolStr::from(&source[name_start..p]);
        if seen.insert(name.clone()) {
            out.push(name);
        }
        i = p;
    }
}

struct WalkCtx<'src> {
    source: &'src str,
}

#[derive(Default)]
struct Counters {
    action_attrs: usize,
    bind_pair: usize,
}

fn walk_fragment(
    fragment: &Fragment,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    for node in &fragment.nodes {
        walk_node(node, summary, counters, ctx);
    }
}

fn walk_node(
    node: &Node,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    match node {
        Node::Element(e) => {
            walk_attributes(&e.attributes, summary, counters, ctx);
            walk_fragment(&e.children, summary, counters, ctx);
        }
        Node::Component(c) => {
            walk_attributes(&c.attributes, summary, counters, ctx);
            collect_component_instantiation(c, ctx.source, summary);
            walk_fragment(&c.children, summary, counters, ctx);
        }
        Node::SvelteElement(s) => {
            walk_attributes(&s.attributes, summary, counters, ctx);
            walk_fragment(&s.children, summary, counters, ctx);
        }
        Node::IfBlock(b) => {
            walk_fragment(&b.consequent, summary, counters, ctx);
            for arm in &b.elseif_arms {
                walk_fragment(&arm.body, summary, counters, ctx);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters, ctx);
            }
        }
        Node::EachBlock(b) => {
            summary.each_block_count += 1;
            walk_fragment(&b.body, summary, counters, ctx);
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters, ctx);
            }
        }
        Node::AwaitBlock(b) => {
            if let Some(p) = &b.pending {
                walk_fragment(p, summary, counters, ctx);
            }
            if let Some(t) = &b.then_branch {
                walk_fragment(&t.body, summary, counters, ctx);
            }
            if let Some(c) = &b.catch_branch {
                walk_fragment(&c.body, summary, counters, ctx);
            }
        }
        Node::KeyBlock(b) => walk_fragment(&b.body, summary, counters, ctx),
        Node::SnippetBlock(b) => walk_fragment(&b.body, summary, counters, ctx),
        // Leaf nodes — no children to descend into, no attributes.
        Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
    }
}

fn walk_attributes(
    attrs: &[Attribute],
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    for attr in attrs {
        if let Attribute::Directive(d) = attr {
            walk_directive(d, summary, counters, ctx);
        }
    }
}

fn walk_directive(
    d: &Directive,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    match d.kind {
        DirectiveKind::Use => {
            let name = format!("__svn_action_attrs_{}", counters.action_attrs);
            summary.void_refs.register(name);
            counters.action_attrs += 1;
        }
        DirectiveKind::Bind => match &d.value {
            Some(DirectiveValue::BindPair { .. }) => {
                let name = format!("__svn_bind_pair_{}", counters.bind_pair);
                summary.void_refs.register(name);
                counters.bind_pair += 1;
            }
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                // `bind:this={x}` and `bind:foo={x}` (any prop name) — if
                // the bound value is a simple identifier, that local
                // gets assigned asynchronously by Svelte (bind:this when
                // the element mounts; bind:foo when the child component
                // updates the bound prop). Record it for the definite-
                // assignment rewrite so closures reading the variable
                // don't fire TS2454.
                if let Some(name) = simple_identifier_in(ctx.source, *expression_range) {
                    summary.bind_this_targets.push(BindThisTarget {
                        name,
                        range: *expression_range,
                    });
                }
                // If the binding name is in our DOM-binding type
                // table (contentRect, contentBoxSize, buffered, …),
                // record the value range + its target type so the
                // emit can generate `<x> = __svn_any() as <TYPE>;`
                // in the template-check body. Catches shapes like
                // `<div bind:contentRect={rect}>` where `rect`'s
                // declared type doesn't accept DOMRectReadOnly.
                //
                // This runs IN ADDITION to the bind-target record
                // above — the same variable needs BOTH the
                // definite-assignment `!` rewrite (assignment is
                // hidden inside a lifecycle callback, flow analysis
                // can't see it) AND the type-compatibility check.
                if let Some(type_annotation) = crate::dom_binding::type_for(d.name.as_str()) {
                    summary.dom_bindings.push(DomBinding {
                        expression: DomBindingExpression::Range(*expression_range),
                        type_annotation,
                    });
                }
            }
            None => {
                // Bare `bind:foo` is shorthand for `bind:foo={foo}` —
                // same definite-assignment story as the explicit form.
                summary.bind_this_targets.push(BindThisTarget {
                    name: d.name.clone(),
                    range: d.range,
                });
                // Also thread through the DOM-binding type check for
                // bare shorthands like `<video bind:buffered>` which
                // desugar to `bind:buffered={buffered}`.
                if let Some(type_annotation) = crate::dom_binding::type_for(d.name.as_str()) {
                    summary.dom_bindings.push(DomBinding {
                        expression: DomBindingExpression::Identifier(d.name.clone()),
                        type_annotation,
                    });
                }
            }
            _ => {}
        },
        _ => {}
    }
}

/// If the byte range covers a single ECMAScript identifier (with optional
/// surrounding whitespace), return it.
fn simple_identifier_in(source: &str, range: Range) -> Option<SmolStr> {
    let slice = source.get(range.start as usize..range.end as usize)?.trim();
    if slice.is_empty() {
        return None;
    }
    let mut chars = slice.chars();
    let first = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    if chars.all(is_ident_continue) {
        Some(SmolStr::from(slice))
    } else {
        None
    }
}

#[inline]
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

#[inline]
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
}

/// Inspect a `<Component ...>` site and, if it's a shape we know how to
/// generate a satisfies-check for, push a `ComponentInstantiation` to
/// the summary.
///
/// Only emits when the component name is a simple identifier (no
/// dotted forms like `<ui.MyButton />` — `typeof ui.MyButton` would
/// require careful expression emission which v0.1 punts on).
///
/// Each plain attribute (including boolean shorthand and `{shorthand}`)
/// contributes a `PropShape` to the literal. Directive attributes
/// (`bind:`, `on:`, `use:`, `class:`, `style:`, `transition:`, etc.)
/// and spreads (`{...obj}`) are silently DROPPED — they provide
/// runtime values that we can't model statically. Their absence from
/// the literal is harmless because emit wraps the satisfies target
/// in `Partial<>`, so missing-required-prop never fires; only the
/// explicit props the user wrote get checked for excess.
///
/// One disqualifier remains: a plain attribute with a multi-part value
/// (`class="a {b} c"`-style interpolation in a quoted attr). The value
/// isn't representable as a single TS expression without re-emitting
/// the template's interpolation pipeline; the whole instantiation is
/// skipped so the satisfies object stays correct on the rest.
fn collect_component_instantiation(
    c: &svn_parser::Component,
    source: &str,
    summary: &mut TemplateSummary,
) {
    if c.name.contains('.') {
        return;
    }
    let mut props: Vec<PropShape> = Vec::with_capacity(c.attributes.len());
    let mut on_events: Vec<OnEventDirective> = Vec::new();
    let mut bind_this_target: Option<SmolStr> = None;
    // Detect "implicit children": any non-snippet, non-whitespace
    // child node between the open/close tags. Pure `{#snippet}`
    // children hoist as explicit props (different code path); pure
    // whitespace (formatting indent) is ignored.
    let has_implicit_children = c.children.nodes.iter().any(|n| match n {
        Node::SnippetBlock(_) => false,
        Node::Text(t) => !t.content.trim().is_empty(),
        _ => true,
    });
    for attr in &c.attributes {
        match attr {
            Attribute::Plain(p) => {
                // SVELTE-4-COMPAT: `slot="x"` on a component is a
                // POSITIONAL marker for the Svelte compiler (places
                // the child into a named slot of its parent), not a
                // prop of the child itself. Emitting it as a prop
                // fires TS2353 on every Svelte-5 child that doesn't
                // declare a `slot` prop. Skip entirely — the
                // Svelte-4 widen on the ENCLOSING parent already
                // handles the case where `slot` *is* explicitly
                // passed as a prop name.
                if p.name.as_str() == "slot" {
                    continue;
                }
                let Some(v) = &p.value else {
                    props.push(PropShape::BoolShorthand {
                        name: p.name.clone(),
                    });
                    continue;
                };
                // Single literal text part (no interpolations) — keep it.
                if v.parts.len() == 1 {
                    if let AttrValuePart::Text { content, .. } = &v.parts[0] {
                        props.push(PropShape::Literal {
                            name: p.name.clone(),
                            value: content.clone(),
                        });
                        continue;
                    }
                }
                // Multi-part interpolated attribute value — disqualify the
                // whole component for this pass.
                return;
            }
            Attribute::Expression(e) => {
                props.push(PropShape::Expression {
                    name: e.name.clone(),
                    expr_range: e.expression_range,
                });
            }
            Attribute::Shorthand(s) => {
                props.push(PropShape::Shorthand {
                    name: s.name.clone(),
                });
            }
            Attribute::Directive(d) => {
                // `on:event={handler}` on a component emits as
                // `$inst.$on("event", handler)` after construction
                // (mirrors upstream svelte2tsx). Handler's type
                // flows through `SvelteComponent<P, E, S>.$on`
                // against the declared Events type.
                if d.kind == svn_parser::DirectiveKind::On {
                    if let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                    {
                        on_events.push(OnEventDirective {
                            event_name: d.name.clone(),
                            handler_range: *expression_range,
                        });
                    }
                    // `on:event` with no value is a bare re-dispatch
                    // — runtime-only, no handler to type-check.
                    continue;
                }
                // `bind:NAME={x}` on a component (other than
                // `bind:this`) is type-equivalent to passing `x` as
                // the `NAME` prop. Emit as a regular expression prop
                // so the child's `Props.NAME` declared type catches
                // mismatches (`<Child bind:value={x: string}>` when
                // `value` is declared `number` fires TS2322).
                //
                // `bind:this={x}` records the identifier so emit can
                // assign the component instance to it — `x =
                // $$_inst;` — after construction. TS checks `x`'s
                // declared type accepts the instance; mismatches
                // fire TS2322 ("Type 'MyComp' is not assignable to
                // type 'OtherComp'"). The existing definite-assign
                // `!` rewrite still fires via `walk_attributes`
                // (that runs first, on every attribute of every
                // node); this captures the instance variable name
                // in addition so the component-local emit can
                // reference it.
                //
                // `bind:GETSET={getter, setter}` (Svelte 5's
                // two-function bind form) is unmodeled here — the
                // expression is a SequenceExpression, not a
                // plain identifier. Fall through with no prop
                // emission; upstream svelte2tsx uses a custom
                // `__sveltets_2_get_set_binding` helper that we
                // haven't ported yet.
                if d.kind == svn_parser::DirectiveKind::Bind && d.name.as_str() == "this" {
                    if let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                        && let Some(id) = simple_identifier_in(source, *expression_range)
                    {
                        bind_this_target = Some(id);
                    }
                    continue;
                }
                if d.kind == svn_parser::DirectiveKind::Bind
                    && let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                {
                    let target = d.name.clone();
                    // Dedup: if the user already wrote the same name
                    // as a plain attribute (`<Child value={x}
                    // bind:value={x} />` — redundant but seen), drop
                    // the prior entry so the bind: expression is the
                    // final authority.
                    props.retain(|p| match p {
                        PropShape::BoolShorthand { name }
                        | PropShape::Literal { name, .. }
                        | PropShape::Expression { name, .. }
                        | PropShape::Shorthand { name } => name != &target,
                        PropShape::Spread { .. } => true, // spreads pass through
                    });
                    props.push(PropShape::Expression {
                        name: target,
                        expr_range: *expression_range,
                    });
                    continue;
                }
                // Bare shorthand `bind:NAME` desugars to
                // `bind:NAME={NAME}` — emit as a Shorthand prop so
                // phase 5's satisfies sees the required field
                // present. Without this branch a
                // `<CustomFieldModal bind:items />` consumer fails
                // the satisfies with "Property 'items' missing"
                // despite the user correctly binding. Mirrors the
                // explicit-expression arm above.
                if d.kind == svn_parser::DirectiveKind::Bind && d.value.is_none() {
                    let target = d.name.clone();
                    props.retain(|p| match p {
                        PropShape::BoolShorthand { name }
                        | PropShape::Literal { name, .. }
                        | PropShape::Expression { name, .. }
                        | PropShape::Shorthand { name } => name != &target,
                        PropShape::Spread { .. } => true,
                    });
                    props.push(PropShape::Shorthand { name: target });
                    continue;
                }
                // Other directives (`use:`, `class:`, `style:`,
                // transitions, animations) are runtime behaviors
                // with no type-level surface on components. Drop.
            }
            // Spread — silently dropped. The Partial<> wrap in emit
            // means we don't need to model the props it would
            // contribute; we only check what the user wrote explicitly.
            Attribute::Spread(s) => {
                // `<Comp {...rest}>` contributes whatever `rest` holds
                // at runtime. Emit as a spread in the props literal so
                // TS structurally type-checks `rest`'s inferred shape
                // against the declared Props — missing-required-prop
                // errors surface on the spread expression itself
                // (useful user-facing signal). A spread CAN fill
                // required-but-not-named-elsewhere props, which is
                // also why phase 5's `satisfies` trailer is
                // tolerant-by-spread without false-positive.
                props.push(PropShape::Spread {
                    expr_range: s.expression_range,
                });
            }
        }
    }
    summary
        .component_instantiations
        .push(ComponentInstantiation {
            component_root: c.name.clone(),
            props,
            has_implicit_children,
            on_events,
            bind_this_target,
            node_start: c.range.start,
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use svn_parser::{parse_all_template_runs, parse_sections};

    fn walk_str(src: &str) -> TemplateSummary {
        let (doc, errors) = parse_sections(src);
        assert!(errors.is_empty(), "section parse errors: {errors:?}");
        let (fragment, errors) = parse_all_template_runs(src, &doc.template.text_runs);
        assert!(errors.is_empty(), "template parse errors: {errors:?}");
        walk_template(&fragment, src)
    }

    #[test]
    fn bind_this_simple_identifier_recorded() {
        let s = walk_str("<div bind:this={inputEl} />");
        assert_eq!(s.bind_this_targets.len(), 1);
        assert_eq!(s.bind_this_targets[0].name, "inputEl");
    }

    #[test]
    fn bind_this_complex_expression_not_recorded() {
        // member expressions, calls, etc. shouldn't trigger the rewrite.
        let s = walk_str("<div bind:this={refs[0]} />");
        assert!(s.bind_this_targets.is_empty());
    }

    #[test]
    fn bind_this_with_dollar_identifier_recorded() {
        let s = walk_str("<div bind:this={$el} />");
        assert_eq!(s.bind_this_targets.len(), 1);
        assert_eq!(s.bind_this_targets[0].name, "$el");
    }

    #[test]
    fn always_registers_template_check() {
        let s = walk_str("<p>hi</p>");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_tpl_check"))
        );
    }

    #[test]
    fn use_directive_registers_action_attrs() {
        // Each `use:foo` directive needs an `__svn_action_attrs_N` holder
        // declared in the template-check function so its inferred attribute
        // type doesn't go unused.
        let s = walk_str(r#"<div use:tooltip={{ text: 'hi' }}>x</div>"#);
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }

    #[test]
    fn multiple_use_directives_get_unique_indices() {
        let s = walk_str(r#"<div use:a use:b><span use:c /></div>"#);
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_1"))
        );
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_2"))
        );
    }

    #[test]
    fn bind_pair_registers_bind_pair() {
        // `bind:foo={getter, setter}` declares a tuple holder; without a
        // void-reference, TypeScript flags it as unused.
        let s = walk_str("<input bind:value={() => g(), (v) => s(v)} />");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_bind_pair_0"))
        );
    }

    #[test]
    fn simple_bind_does_not_register_bind_pair() {
        let s = walk_str("<input bind:value={x} />");
        assert!(
            !s.void_refs
                .names()
                .iter()
                .any(|n| n.starts_with("__svn_bind_pair"))
        );
    }

    #[test]
    fn each_block_increments_count() {
        let s = walk_str("{#each items as item}<p>{item}</p>{/each}");
        assert_eq!(s.each_block_count, 1);
    }

    #[test]
    fn nested_each_blocks_counted() {
        let s = walk_str("{#each rows as row}{#each row.items as item}<x />{/each}{/each}");
        assert_eq!(s.each_block_count, 2);
    }

    #[test]
    fn directives_in_nested_elements_are_walked() {
        let s = walk_str("<div><span use:tooltip /></div>");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }

    #[test]
    fn directives_in_block_body_are_walked() {
        let s = walk_str("{#if cond}<div use:focus />{/if}");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }

    #[test]
    fn each_alternate_branch_walked() {
        let s = walk_str("{#each items as i}<x />{:else}<div use:focus />{/each}");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }
}
