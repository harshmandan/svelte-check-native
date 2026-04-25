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
    /// `bind:this={EXPR}` sites on DOM elements — collected in
    /// source/walk order for v0.3 Item 7's source-map post-scan to
    /// pair 1:1 with `__svn_bind_this_check<TAG>(EXPR);` overlay
    /// occurrences. Covers both simple-identifier and member-
    /// expression forms. Component `bind:this` stays on
    /// `ComponentInstantiation.bind_this_target` (different emit path).
    pub bind_this_checks: Vec<BindThisCheck>,
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
    /// One entry per `use:ACTION={PARAMS}` directive encountered. Emit
    /// uses this to generate `__svn_ensure_action(ACTION(__svn_map_element_tag('TAG'), (PARAMS)))`
    /// — a real call that forces TypeScript to contextually type the
    /// PARAMS expression against `ACTION`'s declared second parameter.
    /// Without this, the params expression (often a callback whose
    /// destructure we want type-checked, e.g. `use:enhance={({formData}) => …}`)
    /// emits unanchored and every destructure silently passes as
    /// implicit `any`.
    pub action_directives: Vec<ActionDirective>,
    /// True when any `<ChildComponent on:EVENT />` bare re-dispatch
    /// directive was seen (the `on:EVENT` form with no `={…}` value —
    /// event bubbling). Drives the default-export Props-widen path
    /// in emit: upstream svelte2tsx's event-handler emit for this
    /// shape produces
    /// `__sveltets_2_bubbleEventDef(__sveltets_2_instanceOf(<Child>).$$events_def, '<event>')`,
    /// which contains a `typeof import('./Child.svelte')` self-
    /// reference in Events-as-return. The circular type chain defeats
    /// generic inference in `__sveltets_2_with_any_event<Props, Events,
    /// …>`, falling back to the `Props = {}` default — effectively
    /// widening the exported component's Props to `{}`. Consumers
    /// then silently accept any object-literal `props`, skipping the
    /// TS2353 excess-prop + TS2322 per-field checks that a strict
    /// Props would fire.
    ///
    /// We mirror the effective behavior by emitting
    /// `Component<Record<string, any>>` for the default export
    /// whenever this flag is set, instead of the strict
    /// `Component<$$ComponentProps>` we'd otherwise use.
    pub has_bubbled_component_event: bool,
    /// `<slot {name}={expr}>` sites encountered, grouped by slot name
    /// (default = unnamed slot). Each entry maps the user's attribute
    /// `name` to the source TEXT of its expression. Emit consumes
    /// these to populate the `slots:` field of `$$render`'s return —
    /// upstream-style scope-resolved literal that flows the
    /// component's slot-prop types into `SvelteComponent<…, …, S>`.
    /// Consumer-side `<Comp let:foo>` then reads typed `foo` from
    /// `inst.$$slot_def[slotName].foo`.
    ///
    /// Identifiers that the walker saw as let-bound or each-bound at
    /// the slot site (i.e. shadowed by a template-scope binding) are
    /// SKIPPED from the attrs map — emitting them at module scope
    /// would resolve them to the wrong (module-scope) declaration.
    /// Skipped names fall through to `any` on the consumer side
    /// (matches the pre-port placeholder behavior). Closing those
    /// cases properly needs the full SlotHandler scope-rewrite port
    /// (see `design/slot_handler/PLAN.md`); this minimal version
    /// targets the Sankey-style case where slot attrs reference
    /// module-scope locals only.
    pub slot_defs: Vec<SlotDef>,
}

/// One `<slot [name="X"] [attr1={expr1}] [attr2]>` site captured for
/// emit-side `slots:` literal generation.
#[derive(Debug, Clone)]
pub struct SlotDef {
    /// Slot name from `name="X"`; `"default"` when omitted.
    pub slot_name: SmolStr,
    /// `(attribute_name, expression_text)` pairs. Expression text is
    /// extracted directly from the source for `={expr}` form, or is
    /// the bare identifier for `{name}` shorthand. `name="literal"`
    /// form falls into the literal-string variant. Expressions that
    /// the walker identified as scope-shadowed (referencing a
    /// let-bound or each-bound name in the active scope) are omitted
    /// from this list — those need the full SlotHandler resolver to
    /// emit correctly and would otherwise resolve to the wrong
    /// module-scope declaration.
    pub attrs: Vec<(SmolStr, String)>,
}

/// One `use:NAME={PARAMS}` directive site. Populated by the template
/// walker; consumed by emit to produce the upstream-shaped call:
///
/// ```ts
/// const __svn_action_<index> = __svn_ensure_action(
///     <action_name>(__svn_map_element_tag('<tag_name>'), (<params_range>))
/// );
/// ```
#[derive(Debug, Clone)]
pub struct ActionDirective {
    /// The N in `__svn_action_<N>` — shared counter with the
    /// `__svn_action_attrs_<N>` void-ref registration so the two
    /// synthesized names stay aligned per directive.
    pub index: usize,
    /// The action's identifier (e.g. `enhance`). Actions in Svelte
    /// must be simple local identifiers — no dotted / computed forms
    /// at the directive site.
    pub action_name: SmolStr,
    /// Parent element's tag (e.g. `form`, `div`). `None` for
    /// `<svelte:element>` where the tag is runtime-dynamic — emit
    /// falls back to the generic `HTMLElement` overload.
    pub tag_name: Option<SmolStr>,
    /// Range of the expression in `use:NAME={EXPR}`. `None` for the
    /// bare `use:NAME` form (no `={…}` value); emit just calls the
    /// action with the element and no params.
    pub params_range: Option<Range>,
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
    /// Source byte range of the expression inside `<Comp
    /// bind:this={EXPR}>`. Covers BOTH simple-identifier
    /// (`bind:this={refs}`) and member-expression
    /// (`bind:this={refs.instance}`) forms. Emit writes `<EXPR> =
    /// $$_inst;` after construction to type-check EXPR's declared
    /// type against the component instance.
    ///
    /// Simple-identifier sites ALSO appear in
    /// `TemplateSummary.bind_this_targets` (populated by
    /// `walk_directive`) for the definite-assign `!` rewrite on the
    /// user's `let x: T` declaration — only simple identifiers have
    /// a declaration to rewrite. Member expressions skip that
    /// path; the assignment emitted here is the only type-check
    /// flow for them.
    pub bind_this_target: Option<Range>,
    /// SVELTE-4-COMPAT: `on:event={handler}` directives on this
    /// component. Emit binds each via `$inst.$on("event", handler)`
    /// on the hoisted instance local, mirroring upstream svelte2tsx's
    /// shape. The props object stays free of `on*` keys so we can
    /// drop the `on${string}` union from `__SvnPropsPartial` — which
    /// in turn stops collisions with user props whose names start
    /// with "on" (`oneTouchReaction`, `onVideoMoments`, etc.).
    pub on_events: Vec<OnEventDirective>,
    /// Simple-identifier targets of `bind:NAME={target}` directives
    /// on this component (excluding `bind:this` which lives in
    /// `bind_this_target`). Emit writes
    /// `() => TARGET = __svn_any(null);` as an uncalled-arrow trailer
    /// after the component's `new` expression so TS flow analysis
    /// sees the target as "assigned to any somewhere" — widens its
    /// inferred type enough to model the Svelte runtime's async
    /// prop writeback. Critical for `let target = $state()` with no
    /// initializer: without the trailer, TS binds `$state<T>()`'s
    /// generic to `{}` / `unknown` and downstream reads
    /// (`target.focus()`) fire TS2339 / TS18046. Mirrors upstream
    /// svelte2tsx's `() => x = __sveltets_2_any(null);` shape in
    /// `htmlxtojsx_v2/nodes/InlineComponent.ts`.
    pub component_bind_widen_targets: Vec<SmolStr>,
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
///
/// Each variant carries `attr_range`: the source byte range of the
/// FULL attribute declaration in the user's `<Component …>` site
/// (e.g. `placement="bottom-end"` covers from `p` to the closing
/// `"`). Emit uses this to push a `TokenMapEntry` for the synthetic
/// `"name": value,` text it generates, so tsgo diagnostics on the
/// prop's typed-check (TS2353 "does not exist", TS2322 wrong type)
/// land at the user's source attribute position rather than the
/// nearest preceding component-name token. Closes the per-prop
/// position-drift class on bench files like palacms FieldItem.svelte
/// where errors used to drift to the enclosing component name.
#[derive(Debug, Clone)]
pub enum PropShape {
    /// `name="literal"` — quoted string value with no interpolation.
    Literal {
        name: SmolStr,
        value: String,
        attr_range: Range,
    },
    /// `name={expr}` — emit the expression source verbatim as the value.
    Expression {
        name: SmolStr,
        expr_range: Range,
        attr_range: Range,
    },
    /// `{name}` — shorthand `name={name}`.
    Shorthand { name: SmolStr, attr_range: Range },
    /// `name` (no `=`) — boolean shorthand.
    BoolShorthand { name: SmolStr, attr_range: Range },
    /// `{...expr}` spread — emit as `...(expr)` in the props literal.
    /// TS type-checks the spread's inferred shape against the
    /// destination type; mismatched spreads fire the usual structural
    /// mismatch errors. Unlike named props, spreads can contribute
    /// any subset of the declared Props AND extra keys in a single
    /// expression.
    Spread {
        expr_range: Range,
        attr_range: Range,
    },
    /// Svelte 5 `bind:NAME={getter, setter}` get/set form. Emit
    /// `name: (getter)()` — calling the getter to obtain the bound
    /// value, whose type is what the target Props declaration
    /// checks against via the `__svn_get_set_binding(get, set)`
    /// helper — `T` flows from getter's return, setter's parameter
    /// is checked against it, return value lands in the prop slot.
    /// Mirrors upstream's `__sveltets_2_get_set_binding`
    /// (svelte2tsx/svelte-shims-v4.d.ts:269).
    ///
    /// Without this variant, the reverted pre-v0.3 behavior would
    /// drop the binding entirely, which v0.3's `satisfies` trailer
    /// surfaces as a spurious "missing required prop" on every
    /// Svelte 5 get/set bind consumer.
    GetSetBinding {
        name: SmolStr,
        getter_range: Range,
        setter_range: Range,
        attr_range: Range,
    },
}

impl PropShape {
    /// Source range of the user-side attribute (`name="value"`,
    /// `name={expr}`, `{...spread}`, etc.) — used by emit to push
    /// a TokenMapEntry on the synthesized prop literal so tsgo
    /// diagnostics map to the user's source attribute position.
    pub fn attr_range(&self) -> Range {
        match self {
            PropShape::Literal { attr_range, .. }
            | PropShape::Expression { attr_range, .. }
            | PropShape::Shorthand { attr_range, .. }
            | PropShape::BoolShorthand { attr_range, .. }
            | PropShape::Spread { attr_range, .. }
            | PropShape::GetSetBinding { attr_range, .. } => *attr_range,
        }
    }
}

/// One `bind:this={x}` site where `x` is a simple identifier. Used
/// for the definite-assignment rewrite on the declaration.
#[derive(Debug, Clone)]
pub struct BindThisTarget {
    /// The identifier name `x`.
    pub name: SmolStr,
    /// Source range of the bind expression (the `x` part).
    pub range: Range,
}

/// One `bind:this={EXPR}` site on a DOM element — recorded in walk
/// order for v0.3 Item 7's post-scan source-map pass. EXPR can be
/// a simple identifier (`myDivRef`), a member expression
/// (`refs.input`), or any other lvalue expression. The emit pairs
/// each entry with a `__svn_bind_this_check<TAG>(EXPR);` occurrence
/// in the overlay.
#[derive(Debug, Clone)]
pub struct BindThisCheck {
    /// Source range of the `={EXPR}` value — emit writes this
    /// verbatim into the check call. TokenMapEntry maps the
    /// overlay-side EXPR span back to this.
    pub expression_range: Range,
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
    let ctx = WalkCtx { source };
    let mut shadow = ShadowStack::default();
    walk_fragment(fragment, &mut summary, &mut counters, &ctx, &mut shadow);
    summary
}

/// Extract the binding name from a `{@const NAME = EXPR}` interpolation
/// body. Returns None for destructured patterns (body starts with `{`)
/// or malformed input — both match upstream's behaviour of emitting
/// nothing for those cases.
///
/// Multiple `{@const}` declarations with the same name across the
/// template are deduped via the caller's `seen` set so emit doesn't
/// generate `let X: any;` twice (TS2451 redeclaration).
fn record_at_const_name(
    interp: &svn_parser::Interpolation,
    source: &str,
    seen: &mut std::collections::HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    let start = interp.expression_range.start as usize;
    let end = interp.expression_range.end as usize;
    let Some(body) = source.get(start..end) else {
        return;
    };
    let bytes = body.as_bytes();
    let mut p = 0usize;
    while p < bytes.len()
        && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_' || bytes[p] == b'$')
    {
        p += 1;
    }
    if p == 0 {
        return; // destructure or malformed
    }
    let name = SmolStr::from(&body[..p]);
    if seen.insert(name.clone()) {
        out.push(name);
    }
}

struct WalkCtx<'src> {
    source: &'src str,
}

#[derive(Default)]
struct Counters {
    action_attrs: usize,
    bind_pair: usize,
    /// Names seen from `{@const NAME = …}` interpolations during this
    /// walk. Used to dedup before pushing into
    /// `summary.at_const_names`; the same name declared twice in the
    /// template (legal Svelte) emits a single `let NAME: any;`.
    at_const_seen: std::collections::HashSet<SmolStr>,
}

/// Per-walk template-scope shadow tracker. Names entered into the
/// stack as the walker descends into scope-introducing nodes
/// (`{#each X as Y}` → `Y`; `{:then Y}` → `Y`; `<Comp let:Y>` /
/// `<el let:Y>` → `Y`; `{#snippet name(Y, …)}` — pattern names);
/// popped on exit. Used by `<slot {Y}>` capture to skip slot-attr
/// expressions that reference scope-bound names — emitting those at
/// module scope (where the `slots:` literal lives in `$$render`'s
/// return) would resolve them to the wrong (module-scope)
/// declaration. Skipped names fall through to `any` on the consumer
/// side. Closing those properly needs the full SlotHandler scope-
/// rewrite port (design/slot_handler/PLAN.md §1.2).
#[derive(Default)]
struct ShadowStack {
    names: Vec<SmolStr>,
}

impl ShadowStack {
    fn push_many(&mut self, names: impl IntoIterator<Item = SmolStr>) -> usize {
        let mark = self.names.len();
        self.names.extend(names);
        mark
    }
    fn truncate(&mut self, mark: usize) {
        self.names.truncate(mark);
    }
    fn contains(&self, name: &str) -> bool {
        self.names.iter().any(|n| n == name)
    }
}

fn walk_fragment(
    fragment: &Fragment,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
    shadow: &mut ShadowStack,
) {
    for node in &fragment.nodes {
        walk_node(node, summary, counters, ctx, shadow);
    }
}

fn walk_node(
    node: &Node,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
    shadow: &mut ShadowStack,
) {
    match node {
        Node::Element(e) => {
            walk_attributes(&e.attributes, summary, counters, ctx, Some(e.name.as_str()));
            collect_bind_this_checks(&e.attributes, summary);
            collect_bind_value_bindings(&e.attributes, e.name.as_str(), summary);
            // `<slot [name="X"] [attr=…]>`: capture for emit's `slots:`
            // literal. Walks the attrs and skips any whose
            // expression references a name in the active shadow stack.
            if e.name.as_str() == "slot" {
                collect_slot_def(&e.attributes, ctx.source, shadow, summary);
            }
            // `let:` directives on an element add their bindings into
            // scope for the element's children. `<svelte:fragment
            // slot="x" let:foo>` is the typical case.
            let let_names = collect_let_directive_names_for_shadow(&e.attributes, ctx.source);
            let mark = shadow.push_many(let_names);
            walk_fragment(&e.children, summary, counters, ctx, shadow);
            shadow.truncate(mark);
        }
        Node::Component(c) => {
            // `use:` on a component is nonsensical at the Svelte level
            // (actions attach to DOM elements, not to component
            // instances), but we pass the component name along anyway —
            // emit's shim-side `__svn_map_element_tag(tag: string)`
            // overload resolves unknown tags to `HTMLElement` so the
            // pattern doesn't break the program.
            walk_attributes(&c.attributes, summary, counters, ctx, None);
            collect_component_instantiation(c, ctx.source, summary);
            // `<Comp let:foo>` adds `foo` into scope for the children.
            let let_names = collect_let_directive_names_for_shadow(&c.attributes, ctx.source);
            let mark = shadow.push_many(let_names);
            walk_fragment(&c.children, summary, counters, ctx, shadow);
            shadow.truncate(mark);
        }
        Node::SvelteElement(s) => {
            // `<svelte:element this={dynamic}>` — tag is only known at
            // runtime. Pass None so emit picks the generic HTMLElement
            // overload of __svn_map_element_tag; actions that declare a
            // specific element type will TS2345 against HTMLElement if
            // they require a narrower base, which matches user intent
            // (action narrowness flags dynamic-tag misuse).
            walk_attributes(&s.attributes, summary, counters, ctx, None);
            collect_bind_this_checks(&s.attributes, summary);
            let let_names = collect_let_directive_names_for_shadow(&s.attributes, ctx.source);
            let mark = shadow.push_many(let_names);
            walk_fragment(&s.children, summary, counters, ctx, shadow);
            shadow.truncate(mark);
        }
        Node::IfBlock(b) => {
            walk_fragment(&b.consequent, summary, counters, ctx, shadow);
            for arm in &b.elseif_arms {
                walk_fragment(&arm.body, summary, counters, ctx, shadow);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters, ctx, shadow);
            }
        }
        Node::EachBlock(b) => {
            summary.each_block_count += 1;
            // `{#each items as item, i (key)}` — `item` and `i` enter
            // scope for the body. The pattern in `context_range` may
            // be a destructure (`as { a, b }`); we collect the
            // top-level identifier names via a simple identifier-
            // extraction pass over the source slice.
            let mut binding_names: Vec<SmolStr> = Vec::new();
            if let Some(as_clause) = &b.as_clause {
                collect_pattern_idents(ctx.source, as_clause.context_range, &mut binding_names);
                if let Some(idx) = &as_clause.index_range {
                    collect_pattern_idents(ctx.source, *idx, &mut binding_names);
                }
            }
            let mark = shadow.push_many(binding_names);
            walk_fragment(&b.body, summary, counters, ctx, shadow);
            shadow.truncate(mark);
            if let Some(alt) = &b.alternate {
                // `{:else}` branch — empty-list body. No bindings in
                // scope (the `as` binding doesn't apply here).
                walk_fragment(alt, summary, counters, ctx, shadow);
            }
        }
        Node::AwaitBlock(b) => {
            if let Some(p) = &b.pending {
                walk_fragment(p, summary, counters, ctx, shadow);
            }
            if let Some(t) = &b.then_branch {
                let mut names: Vec<SmolStr> = Vec::new();
                if let Some(ctx_range) = &t.context_range {
                    collect_pattern_idents(ctx.source, *ctx_range, &mut names);
                }
                let mark = shadow.push_many(names);
                walk_fragment(&t.body, summary, counters, ctx, shadow);
                shadow.truncate(mark);
            }
            if let Some(c) = &b.catch_branch {
                let mut names: Vec<SmolStr> = Vec::new();
                if let Some(ctx_range) = &c.context_range {
                    collect_pattern_idents(ctx.source, *ctx_range, &mut names);
                }
                let mark = shadow.push_many(names);
                walk_fragment(&c.body, summary, counters, ctx, shadow);
                shadow.truncate(mark);
            }
        }
        Node::KeyBlock(b) => walk_fragment(&b.body, summary, counters, ctx, shadow),
        Node::SnippetBlock(b) => {
            // `{#snippet name(p1, p2)}` — params enter scope for the
            // snippet body.
            let mut names: Vec<SmolStr> = Vec::new();
            collect_pattern_idents(ctx.source, b.parameters_range, &mut names);
            let mark = shadow.push_many(names);
            walk_fragment(&b.body, summary, counters, ctx, shadow);
            shadow.truncate(mark);
        }
        Node::Interpolation(i) if i.kind == svn_parser::InterpolationKind::AtConst => {
            record_at_const_name(
                i,
                ctx.source,
                &mut counters.at_const_seen,
                &mut summary.at_const_names,
            );
        }
        // Leaf nodes — no children to descend into, no attributes.
        Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
    }
}

/// Capture a `<slot [name="X"] [attr=…]>` site into
/// `summary.slot_defs`. Skips attrs whose expression references a
/// name in the active shadow stack — those need full scope
/// resolution to emit at module scope correctly. The slot is still
/// recorded (with a possibly-empty attrs list) so consumer-side
/// `<Comp let:foo>` destructure has SOMETHING to read from
/// `inst.$$slot_def[name]`.
fn collect_slot_def(
    attrs: &[Attribute],
    source: &str,
    shadow: &ShadowStack,
    summary: &mut TemplateSummary,
) {
    use svn_parser::{AttrValuePart, Attribute as A};
    let mut slot_name = SmolStr::new("default");
    let mut entries: Vec<(SmolStr, String)> = Vec::new();
    for attr in attrs {
        match attr {
            A::Plain(p) if p.name.as_str() == "name" => {
                if let Some(v) = &p.value
                    && v.parts.len() == 1
                    && let AttrValuePart::Text { content, .. } = &v.parts[0]
                {
                    slot_name = SmolStr::from(content.as_str());
                }
            }
            A::Expression(e) => {
                let start = e.expression_range.start as usize;
                let end = e.expression_range.end as usize;
                let Some(text) = source.get(start..end) else {
                    continue;
                };
                let trimmed = text.trim();
                if is_simple_identifier(trimmed) && shadow.contains(trimmed) {
                    continue;
                }
                entries.push((e.name.clone(), text.to_string()));
            }
            A::Shorthand(s) => {
                if shadow.contains(s.name.as_str()) {
                    continue;
                }
                entries.push((s.name.clone(), s.name.to_string()));
            }
            // Plain literal attrs on `<slot>` (other than `name=`)
            // are unusual; skip them for now. Spread, directives:
            // also skip — full slot-handler port territory.
            _ => {}
        }
    }
    summary.slot_defs.push(SlotDef {
        slot_name,
        attrs: entries,
    });
}

fn is_simple_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' || c == '$' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

/// Extract every let:NAME / let:NAME={alias} binding name from a set
/// of attributes — twin of emit's `collect_let_directive_names`,
/// scoped here for the shadow stack. Identifiers ONLY (no
/// destructure shapes); the bare directive `name` is used as a
/// fallback when the alias isn't a simple identifier.
fn collect_let_directive_names_for_shadow(attrs: &[Attribute], source: &str) -> Vec<SmolStr> {
    use svn_parser::{Directive, DirectiveKind, DirectiveValue};
    let mut out: Vec<SmolStr> = Vec::new();
    for attr in attrs {
        if let Attribute::Directive(Directive {
            kind: DirectiveKind::Let,
            name,
            value,
            ..
        }) = attr
        {
            let bound = match value {
                Some(DirectiveValue::Expression {
                    expression_range, ..
                }) => {
                    let start = expression_range.start as usize;
                    let end = expression_range.end as usize;
                    let slice = source.get(start..end).unwrap_or("").trim();
                    if is_simple_identifier(slice) {
                        SmolStr::from(slice)
                    } else {
                        // Destructure pattern: extract top-level
                        // identifier names so each one shadows.
                        let mut names: Vec<SmolStr> = Vec::new();
                        collect_pattern_idents(source, *expression_range, &mut names);
                        if !names.is_empty() {
                            out.extend(names);
                            continue;
                        }
                        name.clone()
                    }
                }
                _ => name.clone(),
            };
            if !out.iter().any(|n| n == &bound) {
                out.push(bound);
            }
        }
    }
    out
}

/// Extract every simple-identifier name from a binding-pattern byte
/// range. Handles `as item`, `as item, i`, `as { a, b }`, `as [x,
/// y]`, and the snippet param list `(a, b: T, { c })`. Used by the
/// shadow stack to know which names are scope-bound at any point in
/// the walk.
///
/// Implementation: oxc parse of the slice as an arrow-function
/// pattern (the closest parse shape that accepts both destructures
/// and identifiers). On parse failure, falls back to the SmolStr
/// of the trimmed source slice IF it's a simple identifier.
fn collect_pattern_idents(source: &str, range: svn_core::Range, out: &mut Vec<SmolStr>) {
    let start = range.start as usize;
    let end = range.end as usize;
    let Some(slice) = source.get(start..end) else {
        return;
    };
    let trimmed = slice.trim();
    if trimmed.is_empty() {
        return;
    }
    // Fast path: bare identifier.
    if is_simple_identifier(trimmed) {
        out.push(SmolStr::from(trimmed));
        return;
    }
    // Use oxc to parse the slice as a JS expression in the shape
    // of an arrow-function parameter list. Wrap in `(` … `) => 0`.
    let alloc = oxc_allocator::Allocator::default();
    let wrapped = format!("({trimmed}) => 0");
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return;
    }
    use oxc_ast::ast::{BindingPatternKind, Expression, Statement};
    let Some(stmt) = parsed.program.body.first() else {
        return;
    };
    let Statement::ExpressionStatement(es) = stmt else {
        return;
    };
    let Expression::ArrowFunctionExpression(arrow) = &es.expression else {
        return;
    };
    for param in &arrow.params.items {
        collect_from_pattern(&param.pattern.kind, out);
    }

    fn collect_from_pattern(pat: &BindingPatternKind<'_>, out: &mut Vec<SmolStr>) {
        match pat {
            BindingPatternKind::BindingIdentifier(id) => {
                out.push(SmolStr::from(id.name.as_str()));
            }
            BindingPatternKind::ObjectPattern(obj) => {
                for p in &obj.properties {
                    collect_from_pattern(&p.value.kind, out);
                }
                if let Some(rest) = &obj.rest {
                    collect_from_pattern(&rest.argument.kind, out);
                }
            }
            BindingPatternKind::ArrayPattern(arr) => {
                for el in arr.elements.iter().flatten() {
                    collect_from_pattern(&el.kind, out);
                }
            }
            BindingPatternKind::AssignmentPattern(asn) => {
                collect_from_pattern(&asn.left.kind, out);
            }
        }
    }
}

fn walk_attributes(
    attrs: &[Attribute],
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
    parent_tag: Option<&str>,
) {
    for attr in attrs {
        if let Attribute::Directive(d) = attr {
            walk_directive(d, summary, counters, ctx, parent_tag);
        }
    }
}

/// v0.3 Item 8 extended: record `bind:value={EXPR}` sites with a
/// context-aware target type resolved from the element tag + literal
/// `type="..."` sibling attribute. Pushes a `DomBinding` entry so the
/// existing Item 6 emit path handles the assignment-direction check
/// + source-map post-scan uniformly.
///
/// Dispatch matrix:
/// - `<input type="number">` / `<input type="range">` → `number`
/// - `<input type="file">`   → SKIP (`bind:files` is the typed path)
/// - `<input>` any other / no type attribute → `string`
/// - `<textarea>` → `string`
/// - other tags (including `<select>`) → SKIP (upstream dispatches
///   via `svelteHTML.createElement` ambient typing; we don't have
///   that wired in, so staying silent matches upstream's "pass"
///   behavior at typecheck level on these cases).
///
/// `bind:group` is intentionally NOT recorded — upstream widens the
/// target to `any` (`__sveltets_2_any(null)`), we simply skip the
/// check entirely which has the same observable no-error outcome.
fn collect_bind_value_bindings(attrs: &[Attribute], tag_name: &str, summary: &mut TemplateSummary) {
    let Some(ty) = resolve_bind_value_type(tag_name, attrs) else {
        return;
    };
    for attr in attrs {
        let Attribute::Directive(d) = attr else {
            continue;
        };
        if d.kind != svn_parser::DirectiveKind::Bind || d.name.as_str() != "value" {
            continue;
        }
        let expression = match &d.value {
            Some(svn_parser::DirectiveValue::Expression {
                expression_range, ..
            }) => DomBindingExpression::Range(*expression_range),
            None => DomBindingExpression::Identifier(d.name.clone()),
            _ => continue,
        };
        summary.dom_bindings.push(DomBinding {
            expression,
            type_annotation: ty,
        });
    }
}

/// Dispatch the target type for a `bind:value` directive based on the
/// element tag + literal `type="..."` sibling attribute. Shared by
/// analyze (collection into `summary.dom_bindings`) and emit
/// (inline contract-check generation) so both pipelines stay in sync.
///
/// Returns `None` for tags / type-attr combinations we don't model:
/// - `<input type="file" | "checkbox" | "radio">`: handled by
///   `bind:files` / `bind:checked` (different table entries).
/// - `<select>`: target type depends on `<option>` values; not
///   statically resolvable without option inspection.
/// - Other tags: `bind:value` isn't meaningful.
pub fn resolve_bind_value_type(tag_name: &str, attrs: &[Attribute]) -> Option<&'static str> {
    match tag_name {
        "input" => match literal_attr_value(attrs, "type") {
            Some("number") | Some("range") => Some("number"),
            Some("file") | Some("checkbox") | Some("radio") => None,
            _ => Some("string"),
        },
        "textarea" => Some("string"),
        _ => None,
    }
}

/// Return the literal string value of a plain attribute `name="LITERAL"`,
/// or None if the attribute is absent, quoted with an expression
/// interpolation, or bound via `name={expr}`. Used for context-aware
/// bind dispatch (`<input type="number" bind:value={...}>`).
pub fn literal_attr_value<'a>(attrs: &'a [Attribute], name: &str) -> Option<&'a str> {
    for attr in attrs {
        let Attribute::Plain(p) = attr else {
            continue;
        };
        if p.name.as_str() != name {
            continue;
        }
        let value = p.value.as_ref()?;
        // Require a single text part — reject interpolated values like
        // `type="my-{x}"` where we can't statically resolve the type.
        let [svn_parser::AttrValuePart::Text { content, .. }] = value.parts.as_slice() else {
            return None;
        };
        return Some(content.as_str());
    }
    None
}

/// v0.3 Item 7: record `bind:this={EXPR}` sites on DOM elements and
/// `<svelte:element>` for emit's source-map post-scan. Emit pairs
/// each entry with a `__svn_bind_this_check<TAG>(EXPR);` overlay
/// occurrence and pushes a TokenMapEntry. Walk order matches emit
/// order; pairing is N-th to N-th.
fn collect_bind_this_checks(attrs: &[Attribute], summary: &mut TemplateSummary) {
    for attr in attrs {
        let Attribute::Directive(d) = attr else {
            continue;
        };
        if d.kind != svn_parser::DirectiveKind::Bind || d.name.as_str() != "this" {
            continue;
        }
        let Some(svn_parser::DirectiveValue::Expression {
            expression_range, ..
        }) = &d.value
        else {
            continue;
        };
        summary.bind_this_checks.push(BindThisCheck {
            expression_range: *expression_range,
        });
    }
}

fn walk_directive(
    d: &Directive,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
    parent_tag: Option<&str>,
) {
    match d.kind {
        DirectiveKind::Use => {
            let index = counters.action_attrs;
            let name = format!("__svn_action_attrs_{index}");
            summary.void_refs.register(name);
            counters.action_attrs += 1;
            // Capture the full action-directive shape so emit can build
            // the real call — `action(element, params)` — rather than
            // the pre-v0.3.9 placeholder that dropped both sides and
            // lost contextual typing on the params expression.
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
/// Handles both simple component names (`<MyButton />`) and dotted
/// forms (`<ui.MyButton />`, `<UI.TextInput>`). The dotted form is
/// passed verbatim through to the emit's `__svn_ensure_component(...)`
/// call — `UI.TextInput` evaluates to the member-referenced component
/// value, which the ensure_component overloads resolve the same way
/// as a simple-identifier reference. Emit also voids the root
/// identifier via template_refs, so the barrel import isn't flagged
/// unused.
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
    let mut props: Vec<PropShape> = Vec::with_capacity(c.attributes.len());
    let mut on_events: Vec<OnEventDirective> = Vec::new();
    let mut bind_this_target: Option<Range> = None;
    let mut component_bind_widen_targets: Vec<SmolStr> = Vec::new();
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
                        attr_range: p.range,
                    });
                    continue;
                };
                // Single literal text part (no interpolations) — keep it.
                if v.parts.len() == 1 {
                    if let AttrValuePart::Text { content, .. } = &v.parts[0] {
                        props.push(PropShape::Literal {
                            name: p.name.clone(),
                            value: content.clone(),
                            attr_range: p.range,
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
                    attr_range: e.range,
                });
            }
            Attribute::Shorthand(s) => {
                props.push(PropShape::Shorthand {
                    name: s.name.clone(),
                    attr_range: s.range,
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
                    } else {
                        // `on:event` with no value — bare re-dispatch
                        // (event bubbling from sub-component). Flag at
                        // summary level so emit can widen this
                        // component's own default-export Props to
                        // match upstream's `with_any_event` +
                        // `isomorphic_component` inference-failure
                        // widening (see
                        // `TemplateSummary::has_bubbled_component_event`
                        // docs for the mechanism).
                        summary.has_bubbled_component_event = true;
                    }
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
                    {
                        // Record the full expression range regardless
                        // of shape (simple identifier OR member
                        // expression). Emit renders source verbatim.
                        // The sibling `bind_this_targets` collection
                        // via `walk_directive` still filters for
                        // simple-identifier names — that's for the
                        // declaration-site `!` rewrite which only
                        // applies to simple `let` declarations.
                        bind_this_target = Some(*expression_range);
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
                        PropShape::BoolShorthand { name, .. }
                        | PropShape::Literal { name, .. }
                        | PropShape::Expression { name, .. }
                        | PropShape::Shorthand { name, .. }
                        | PropShape::GetSetBinding { name, .. } => name != &target,
                        PropShape::Spread { .. } => true, // spreads pass through
                    });
                    props.push(PropShape::Expression {
                        name: target,
                        expr_range: *expression_range,
                        attr_range: d.range,
                    });
                    // Widen target if the expression is a simple
                    // identifier — emit's post-`new` trailer will write
                    // `() => <ident> = __svn_any(null);` so TS flow
                    // analysis widens the target's type to `any`. Only
                    // simple identifiers are safe; member expressions
                    // (`bind:prop={x.y}`) and destructures aren't
                    // assignable in a one-liner arrow without matching
                    // the exact declaration shape.
                    if let Some(ident) = simple_identifier_in(source, *expression_range) {
                        component_bind_widen_targets.push(ident);
                    }
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
                        PropShape::BoolShorthand { name, .. }
                        | PropShape::Literal { name, .. }
                        | PropShape::Expression { name, .. }
                        | PropShape::Shorthand { name, .. }
                        | PropShape::GetSetBinding { name, .. } => name != &target,
                        PropShape::Spread { .. } => true,
                    });
                    // Bare `bind:NAME` is `bind:NAME={NAME}` — same
                    // widening trailer as the explicit form.
                    component_bind_widen_targets.push(target.clone());
                    props.push(PropShape::Shorthand {
                        name: target,
                        attr_range: d.range,
                    });
                    continue;
                }
                // Svelte 5 `bind:NAME={getter, setter}` get/set form
                // (DirectiveValue::BindPair). Upstream svelte2tsx uses
                // a `__sveltets_2_get_set_binding` helper to model
                // it; we don't yet. Without this branch v0.3's
                // satisfies trailer catches `name` / `definition`
                // etc. as "missing required props" on consumers that
                // correctly use the get/set form — false positives
                // on Svelte 5 bind: idiom.
                //
                // Interim: push the name as Shorthand so satisfies
                // Emit site lowers this to `name: __svn_get_set_binding(
                // <getter>, <setter>)` — mirrors upstream's
                // `__sveltets_2_get_set_binding` helper so TS infers `T`
                // from the getter's return AND checks the setter's
                // parameter against the same `T`. See
                // `design/get_set_binding/` for the fixture-locked shape.
                if d.kind == svn_parser::DirectiveKind::Bind
                    && let Some(svn_parser::DirectiveValue::BindPair {
                        getter_range,
                        setter_range,
                        ..
                    }) = &d.value
                {
                    let target = d.name.clone();
                    props.retain(|p| match p {
                        PropShape::BoolShorthand { name, .. }
                        | PropShape::Literal { name, .. }
                        | PropShape::Expression { name, .. }
                        | PropShape::Shorthand { name, .. }
                        | PropShape::GetSetBinding { name, .. } => name != &target,
                        PropShape::Spread { .. } => true,
                    });
                    props.push(PropShape::GetSetBinding {
                        name: target,
                        getter_range: *getter_range,
                        setter_range: *setter_range,
                        attr_range: d.range,
                    });
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
                    attr_range: s.range,
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
            component_bind_widen_targets,
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
