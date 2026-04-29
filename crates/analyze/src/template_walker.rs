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

use smallvec::SmallVec;
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
    pub bind_this_targets: SmallVec<BindThisTarget, 2>,
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
    pub at_const_names: SmallVec<SmolStr, 4>,
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
    pub component_instantiations: SmallVec<ComponentInstantiation, 4>,
    /// One entry per `use:ACTION={PARAMS}` directive encountered. Emit
    /// uses this to generate `__svn_ensure_action(ACTION(__svn_map_element_tag('TAG'), (PARAMS)))`
    /// — a real call that forces TypeScript to contextually type the
    /// PARAMS expression against `ACTION`'s declared second parameter.
    /// Without this, the params expression (often a callback whose
    /// destructure we want type-checked, e.g. `use:enhance={({formData}) => …}`)
    /// emits unanchored and every destructure silently passes as
    /// implicit `any`.
    pub action_directives: SmallVec<ActionDirective, 2>,
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
    /// Bare `on:NAME` directives (no `={…}` value) seen on real DOM
    /// elements and on `<svelte:body>` / `<svelte:window>`. At runtime
    /// these forward the native DOM event up to a parent listener; at
    /// type-check time the consumer's `<Child on:NAME={cb}>` should
    /// see the DOM event type (`MouseEvent`, `KeyboardEvent`, …)
    /// rather than `CustomEvent<any>`.
    ///
    /// Emit consumes this list to project a raw DOM-event map
    /// (`{ "click": HTMLElementEventMap["click"], … }`) and
    /// intersects it with the wrapped dispatcher-detail map to form
    /// the FINAL `$$Events` alias. The per-entry `scope` selects the
    /// right event map (`HTMLElementEventMap` / `HTMLBodyElementEventMap`
    /// / `WindowEventMap`) — mirrors upstream svelte2tsx's
    /// `__sveltets_2_mapElementEvent('click')` /
    /// `__sveltets_2_mapBodyEvent` / `__sveltets_2_mapWindowEvent`
    /// dispatch in `event-handler.ts:63-72`.
    ///
    /// Bare `on:NAME` on a `<Component>` does NOT land here — that
    /// path goes through `has_bubbled_component_event` (different
    /// upstream emit, different consumer effect). `<svelte:document>`
    /// is intentionally NOT mapped: upstream svelte2tsx's
    /// `event-handler.ts` only handles Body / Window
    /// (`DocumentEventMap` is omitted there too).
    ///
    /// Names appear in walk order with no dedup — the caller
    /// dedupes when projecting (a duplicate key in a TS object type
    /// is fine but noisy).
    pub bubbled_dom_events: SmallVec<BubbledDomEvent, 2>,
    /// Bare `<Child on:NAME />` directives on components — each
    /// re-dispatches Child's NAME event up to the wrapper's own
    /// consumers. Reviewer follow-up #2: pre-fix the walker only
    /// flagged `has_bubbled_component_event` and recorded the local
    /// `$on(...)` for child-event-name validation, but the wrapper's
    /// own `$$Events` surface didn't carry the bubbled name —
    /// consumers of the wrapper saw nothing.
    ///
    /// Emit projects each entry into `events_alias_body` via
    /// `__SvnComponentEvents<typeof <component_root>>["<name>"]`.
    /// Mirrors upstream svelte2tsx's
    /// `__sveltets_2_bubbleEventDef(__sveltets_2_instanceOf(<Comp>).$$events_def, '<name>')`
    /// projection in `event-handler.ts:55-60`.
    ///
    /// Walk-order preserved; emit dedupes by name.
    pub bubbled_component_events: Vec<BubbledComponentEvent>,
}

/// One bare `<Child on:NAME />` directive — the wrapper's
/// `$$Events` surface should carry NAME with Child's declared event
/// type. Captured in walk order; emit dedupes and projects.
#[derive(Debug, Clone)]
pub struct BubbledComponentEvent {
    pub event_name: SmolStr,
    /// Source-text of the component identifier (or dotted form like
    /// `UI.Dropdown`). Used in emit's `typeof <root>` reference; if
    /// it's a synthetic root (`__svn_self_default` for
    /// `<svelte:self>` / `(<expr>)` for `<svelte:component>`), emit
    /// falls back to `Record<string, any>` for the event type
    /// (matches upstream's any-fallback for dynamic components).
    pub component_root: SmolStr,
    /// Source-byte position of the directive (start of the
    /// `on:NAME` token). Used by emit to merge DOM + component
    /// bubbles in source order with last-wins dedup, matching
    /// upstream's `EventHandler.bubbledEvents.set(...)` semantics.
    pub position: u32,
}

/// Source-element kind for a bubbled `on:NAME` directive — drives the
/// TS event-map name in the `$$Events` projection. Mirrors upstream
/// svelte2tsx `event-handler.ts:63-72`'s switch on the element node
/// type (`Element` / `Body` / `Window`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbledDomEventScope {
    /// Regular DOM element (`<button on:click>`) — emit projects
    /// via `HTMLElementEventMap[NAME]`.
    Element,
    /// `<svelte:body on:click>` — emit projects via
    /// `WindowEventMap[NAME]` (mirrors upstream svelte2tsx's odd
    /// shim convention where `__sveltets_2_mapBodyEvent` returns
    /// `WindowEventMap[K]` — see `svelte-shims.d.ts:185-190`).
    SvelteBody,
    /// `<svelte:window on:resize>` — emit projects via
    /// `HTMLBodyElementEventMap[NAME]` (same upstream-internal
    /// swap as `SvelteBody`).
    SvelteWindow,
}

/// One bare `on:NAME` directive captured for the `$$Events` synth.
#[derive(Debug, Clone)]
pub struct BubbledDomEvent {
    /// Event name without the `on:` prefix (e.g. `click`).
    pub name: SmolStr,
    /// Which event-map the projection draws from.
    pub scope: BubbledDomEventScope,
    /// Source-byte position of the directive (start of the
    /// `on:NAME` token). Used by emit to merge DOM + component
    /// bubbles in source order with last-wins dedup, matching
    /// upstream's `EventHandler.bubbledEvents.set(...)` semantics.
    pub position: u32,
}

/// Expression-text source for one slot attr.
#[derive(Debug, Clone)]
pub enum SlotAttrExpr {
    /// `name={expr}` — emit reads `&source[range]` at splice time
    /// (saves a per-expression heap copy through the analyze
    /// summary).
    Range(Range),
    /// `{name}` shorthand — the bare identifier IS the expression
    /// text. Stored inline as `SmolStr` so we don't recompute the
    /// inner-identifier range from the outer `{name}` range.
    Shorthand(SmolStr),
    /// `name="literal"` quoted text value (no interpolations) —
    /// emits as a TS string literal in the slot-def. Mirrors
    /// upstream svelte2tsx's `SlotHandler` literal-attr branch.
    Literal(String),
    /// Resolved expression — analyze rewrote the user's
    /// scope-shadowed identifier to a scope-independent form (e.g.
    /// `(typeof items extends Iterable<infer __svn_T> ? __svn_T : never)`
    /// for an `{#each items as item}` reference). Two flavors:
    ///
    /// - `Value(s)` → emit splices `(s)` — for value expressions.
    /// - `Type(s)` → emit splices `undefined as any as (s)` — for
    ///   type assertions where no value lives in scope.
    ///
    /// Stage 2 of the SlotHandler port — the each/await resolver
    /// uses this for bindings that don't need the upstream
    /// `__svn_instanceOf` shim. Let-forwarded bindings (Stage 4)
    /// emit `Type("__SvnComponentSlots<typeof C>['default']['name']")`.
    Resolved(ResolvedSlotExpr),
}

/// Output form of a resolved slot-attr expression — see the
/// `Resolved` variant of [`SlotAttrExpr`].
#[derive(Debug, Clone)]
pub enum ResolvedSlotExpr {
    /// A real value expression. Emit writes `(s)`.
    Value(String),
    /// A type assertion. Emit writes `undefined as any as (s)`.
    Type(String),
}

/// One `<slot [name="X"] [attr1={expr1}] [attr2]>` site captured for
/// emit-side `slots:` literal generation.
#[derive(Debug, Clone)]
pub struct SlotDef {
    /// Slot name from `name="X"`; `"default"` when omitted.
    pub slot_name: SmolStr,
    /// Slot attributes in walk order. Attrs whose leading identifier
    /// is shadowed AND can't be resolved through the active scope
    /// (no upstream-equivalent rewrite available) are omitted —
    /// emitting them at module scope would resolve to the wrong
    /// declaration, and a value-fallback would propagate `any`
    /// silently. Stage 2+ of the SlotHandler port progressively
    /// reduces what gets dropped.
    pub attrs: Vec<SlotAttr>,
}

/// One `<slot>` attribute — either a named prop or a spread.
#[derive(Debug, Clone)]
pub enum SlotAttr {
    /// `name={expr}` / `{name}` / `name="lit"` / a resolved attr.
    Prop {
        name: SmolStr,
        expr: SlotAttrExpr,
    },
    /// `{...expr}` — spread of an object's properties. Stage 3 of
    /// the SlotHandler port wires this through; for now nothing
    /// produces it (`collect_slot_def` skips spreads).
    Spread { expr: SlotAttrExpr },
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
    /// Source range of the event NAME itself in the directive's
    /// source slice — used by emit to push a TokenMapEntry on the
    /// synthesised `"NAME"` string literal. Reviewer follow-up #1
    /// added this so TS2345 firing on a bubbled-event name the child
    /// doesn't declare maps back to the source `on:NAME` position
    /// rather than getting filtered by the synth-scaffolding source-
    /// map filter.
    pub name_range: Range,
    /// Source range of the handler expression. Empty (start == end)
    /// when the user wrote `on:event` with no `={…}` value — bare
    /// re-dispatch shorthand (event bubbling). Emit substitutes
    /// `() => {}` as the handler so the `$on` overload still
    /// resolves and the event name's `keyof Events` constraint
    /// fires.
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
    /// Opaque value — emit `name: __svn_any()` so TS sees the prop
    /// is present without trying to model its type. Used today for
    /// multi-part interpolated quoted attrs like
    /// `class="a {b} c"`: a quoted attribute value with one or more
    /// interpolations. Emit assembles a TS template literal —
    /// `\`a ${b} c\`` — so the prop's value carries a real string
    /// type (with the embedded expressions type-checked through
    /// contextual typing inside `${…}`). Mirrors upstream
    /// svelte2tsx's `Attribute.ts:233` template-literal branch.
    ///
    /// `parts` is the parser's per-chunk decomposition (text vs
    /// expression). Walk-order preserved so emit's reassembly
    /// matches the source layout.
    TemplateLiteral {
        name: SmolStr,
        parts: Vec<AttrValuePart>,
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
            | PropShape::GetSetBinding { attr_range, .. }
            | PropShape::TemplateLiteral { attr_range, .. } => *attr_range,
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
///
/// Drives an internal [`AnalyzeVisitor`] over the unified
/// [`crate::template_scope::walk_with_visitor`] walker. The visitor
/// owns `TemplateSummary`, `Counters`, and the analyze-side
/// `ShadowStack`; the walker handles recursion + scope bracketing.
pub fn walk_template(fragment: &Fragment, source: &str) -> TemplateSummary {
    let summary = TemplateSummary::default();
    // Note: the template-check wrapper is now an arrow-expression
    // statement (`;(async () => {...});`) rather than a named
    // function declaration, so there's no identifier to void.
    // Kept for narrowing reasons — see emit's render_function.rs.
    let mut visitor = AnalyzeVisitor {
        summary,
        counters: Counters::default(),
        source,
        shadow: ResolverStack::default(),
        scope_marks: Vec::new(),
        pending_each_items_range: None,
        pending_await_promise_range: None,
        pending_let_owner: None,
    };
    crate::template_scope::walk_with_visitor(fragment, source, &mut visitor);
    visitor.summary
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

/// Per-walk resolver stack — replaces the older shadow-stack-of-
/// names model with one that ALSO carries each binding's
/// `Option<ResolvedSlotExpr>` so slot-attr collection can rewrite
/// bare references to scope-independent forms.
///
/// Three states per name when looked up:
///   - `Some(Some(expr))` — bound and resolvable. Slot attrs
///     splice `expr` directly. Stage-2 each/await bindings use
///     this; Stage-4 let-forwarded bindings will too.
///   - `Some(None)` — bound but NOT safely resolvable (let-
///     directive / snippet param without upstream-equivalent
///     resolution). Slot attrs DROP — emitting at module scope
///     would resolve to the wrong declaration. Closes those
///     properly when the resolver covers the binding kind.
///   - `None` (missing from stack) — module / import / prop
///     identifier. Slot attrs splice source verbatim.
///
/// Walks innermost-first via reverse iteration so a let-binding
/// inside an `{#each}` overrides an outer same-name binding.
///
/// PLAN: design/slot_handler/PLAN.md §3.2.
#[derive(Default)]
struct ResolverStack {
    entries: Vec<(SmolStr, Option<ResolvedSlotExpr>)>,
}

impl ResolverStack {
    fn truncate(&mut self, mark: usize) {
        self.entries.truncate(mark);
    }
    fn lookup(&self, name: &str) -> Option<&Option<ResolvedSlotExpr>> {
        self.entries
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v)
    }
}

/// Per-walk visitor mapping `TemplateScopeVisitor` calls into
/// analyze-side mutations. Domain-level work (attribute collection,
/// bind-this targets, component instantiations, slot-def capture)
/// happens inside the `visit_*` methods; the unified walker drives
/// recursion and scope bracketing.
struct AnalyzeVisitor<'src> {
    summary: TemplateSummary,
    counters: Counters,
    source: &'src str,
    shadow: ResolverStack,
    /// Stack of entry marks pushed by `enter_scope` / `enter_fragment`.
    /// `leave_scope` / `leave_fragment` pop the matching mark and
    /// truncate the resolver stack back to it.
    scope_marks: Vec<usize>,
    /// Source range of the most recent `{#each EXPR as ...}` outer
    /// expression — stashed by `visit_each_block` and consumed by the
    /// next `enter_scope(Each, …)` call. The walker calls
    /// `visit_each_block` immediately before `enter_scope(Each)` so
    /// they're guaranteed paired (per `template_scope::walk_node_inner`
    /// for `Node::EachBlock`).
    pending_each_items_range: Option<Range>,
    /// Same idea, for `{#await EXPR ...}`. Consumed by the next
    /// `enter_scope(AwaitThen, …)` (or AwaitCatch if there's no
    /// then branch). Reset per await-block so each branch picks up
    /// the same promise range.
    pending_await_promise_range: Option<Range>,
    /// SlotHandler PLAN Stage 4: stashed by `visit_component` /
    /// `visit_svelte_element` (Component / SelfRef kinds) and
    /// consumed by the next `enter_scope(LetDirective, …)` call.
    /// When present, `let:foo` bindings on this component resolve
    /// to `__SvnComponentSlots<typeof <root>>['default']['foo']`.
    /// `None` for elements that aren't producer-side let owners
    /// (DOM elements, components with `slot=` consumer wrappers,
    /// dynamic `<svelte:component this={EXPR}>` forms whose root
    /// isn't a typeable identifier).
    pending_let_owner: Option<LetOwnerInfo>,
}

/// Producer-side let-owner info — see
/// `AnalyzeVisitor.pending_let_owner`.
#[derive(Debug, Clone)]
struct LetOwnerInfo {
    /// `typeof <root>`-safe component identifier.
    component_root: SmolStr,
    /// Slot name the let-bindings target. `"default"` unless a
    /// future stage adds named-slot let-forwarding.
    slot_name: SmolStr,
}

impl crate::template_scope::TemplateScopeVisitor for AnalyzeVisitor<'_> {
    fn enter_fragment(&mut self) {
        self.scope_marks.push(self.shadow.entries.len());
    }

    fn leave_fragment(&mut self) {
        if let Some(mark) = self.scope_marks.pop() {
            self.shadow.truncate(mark);
        }
    }

    fn visit_each_block(&mut self, b: &svn_parser::EachBlock) {
        self.summary.each_block_count += 1;
        self.pending_each_items_range = Some(b.expression_range);
    }

    fn visit_await_block(&mut self, b: &svn_parser::AwaitBlock) {
        self.pending_await_promise_range = Some(b.expression_range);
    }

    fn enter_scope(
        &mut self,
        kind: crate::template_scope::ScopeKind,
        bindings: &[crate::template_scope::BoundIdent],
    ) {
        let mark = self.shadow.entries.len();
        match kind {
            crate::template_scope::ScopeKind::Each { has_index, .. } => {
                // Convention from `template_scope`: when `has_index`
                // is true, the index identifier is the LAST binding;
                // every preceding entry is a context (item) binding.
                let items_range = self.pending_each_items_range.take();
                let context_count = if has_index {
                    bindings.len().saturating_sub(1)
                } else {
                    bindings.len()
                };
                let items_text = items_range.and_then(|r| {
                    self.source
                        .get(r.start as usize..r.end as usize)
                        .map(|s| s.trim().to_string())
                });
                for (i, b) in bindings.iter().enumerate() {
                    let resolved = if has_index && i == context_count {
                        // Index — always `number`.
                        Some(ResolvedSlotExpr::Type("number".to_string()))
                    } else if let Some(items) = items_text.as_deref() {
                        // Bare context binding only — destructured
                        // patterns (`{#each rows as { id }}`)
                        // produce multiple bindings whose source
                        // doesn't match the items expression. Stage 3
                        // will rewrite via OXC; for now leave them as
                        // `None` (shadowed but unresolvable).
                        if context_count == 1 {
                            Some(ResolvedSlotExpr::Type(format!(
                                "(typeof {items}) extends Iterable<infer __svn_T> \
                                 ? __svn_T : never"
                            )))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    self.shadow.entries.push((b.name.clone(), resolved));
                }
            }
            crate::template_scope::ScopeKind::AwaitThen => {
                let promise_range = self.pending_await_promise_range;
                let promise_text = promise_range.and_then(|r| {
                    self.source
                        .get(r.start as usize..r.end as usize)
                        .map(|s| s.trim().to_string())
                });
                for b in bindings {
                    // Bare `{:then v}` → `Awaited<typeof p>`.
                    // Destructured `{:then { x }}` falls to None
                    // (Stage 3 OXC rewriter territory).
                    let resolved = if bindings.len() == 1 {
                        promise_text.as_deref().map(|p| {
                            ResolvedSlotExpr::Type(format!("Awaited<typeof {p}>"))
                        })
                    } else {
                        None
                    };
                    self.shadow.entries.push((b.name.clone(), resolved));
                }
            }
            crate::template_scope::ScopeKind::AwaitCatch => {
                // `{:catch e}` — error type is `any` (matches
                // upstream's `resolveExpression` returning
                // `__sveltets_2_any({})`).
                for b in bindings {
                    let resolved = if bindings.len() == 1 {
                        Some(ResolvedSlotExpr::Type("any".to_string()))
                    } else {
                        None
                    };
                    self.shadow.entries.push((b.name.clone(), resolved));
                }
            }
            crate::template_scope::ScopeKind::LetDirective => {
                // SlotHandler PLAN Stage 4: producer-side
                // `<Comp let:foo>` resolves `foo` to
                // `__SvnComponentSlots<typeof Comp>['default']['foo']`.
                // Consume the stash that `visit_component` /
                // `visit_svelte_element` (Component / SelfRef) put
                // there. None means we're inside a non-resolvable
                // case (consumer wrapper with `slot=`, dynamic
                // `<svelte:component this={EXPR}>` whose root isn't
                // a typeable identifier, plain DOM element with
                // `let:`); fall through as unresolvable so the
                // slot-attr collector drops references rather than
                // splicing module scope.
                let owner = self.pending_let_owner.take();
                for b in bindings {
                    let resolved = owner.as_ref().map(|info| {
                        ResolvedSlotExpr::Type(format!(
                            "__SvnComponentSlots<typeof {root}>[{slot:?}][{name:?}]",
                            root = info.component_root.as_str(),
                            slot = info.slot_name.as_str(),
                            name = b.name.as_str(),
                        ))
                    });
                    self.shadow.entries.push((b.name.clone(), resolved));
                }
            }
            crate::template_scope::ScopeKind::Snippet
            | crate::template_scope::ScopeKind::Fragment => {
                // Snippet params don't have an upstream-equivalent
                // slot resolution (per PLAN §6 "things not to do").
                // Fragment scope is a bracket boundary — no bindings
                // declared. Both fall through as `None`.
                for b in bindings {
                    self.shadow.entries.push((b.name.clone(), None));
                }
            }
        }
        self.scope_marks.push(mark);
    }

    fn leave_scope(&mut self, _kind: crate::template_scope::ScopeKind) {
        if let Some(mark) = self.scope_marks.pop() {
            self.shadow.truncate(mark);
        }
    }

    fn visit_element(&mut self, e: &svn_parser::Element) {
        let ctx = WalkCtx {
            source: self.source,
        };
        walk_attributes(
            &e.attributes,
            &mut self.summary,
            &mut self.counters,
            &ctx,
            Some(e.name.as_str()),
        );
        collect_bind_this_checks(&e.attributes, &mut self.summary);
        collect_bind_value_bindings(&e.attributes, e.name.as_str(), &mut self.summary);
        collect_bubbled_dom_events(
            &e.attributes,
            BubbledDomEventScope::Element,
            &mut self.summary,
        );
        // `<slot [name="X"] [attr=…]>`: capture for emit's `slots:`
        // literal. Walks the attrs and skips any whose expression
        // references a name in the active shadow stack.
        if e.name.as_str() == "slot" {
            collect_slot_def(&e.attributes, self.source, &self.shadow, &mut self.summary);
        }
    }

    fn visit_component(&mut self, c: &svn_parser::Component) {
        let ctx = WalkCtx {
            source: self.source,
        };
        // `use:` on a component is nonsensical at the Svelte level
        // (actions attach to DOM elements, not to component instances),
        // but we pass it along — emit's shim-side
        // `__svn_map_element_tag(tag: string)` overload resolves
        // unknown tags to `HTMLElement` so the pattern doesn't break
        // the program.
        walk_attributes(
            &c.attributes,
            &mut self.summary,
            &mut self.counters,
            &ctx,
            None,
        );
        collect_component_instantiation(c, self.source, &mut self.summary);
        // SlotHandler PLAN Stage 4: stash producer-side let-owner
        // info so the next `enter_scope(LetDirective, …)` can
        // resolve `let:foo` bindings to
        // `__SvnComponentSlots<typeof Comp>['default']['foo']`.
        // Skip when:
        //   - `slot="X"` attr present (consumer-wrapper case —
        //     the let-bindings then target the PARENT's slot,
        //     not this component's; current resolver lacks
        //     parent context, so leave unresolved).
        //   - component name isn't a simple identifier (dotted
        //     forms like `UI.Dropdown` would need a different
        //     `typeof` shape; defer until a fixture proves it).
        let has_slot_attr = literal_attr_value(&c.attributes, "slot").is_some();
        if !has_slot_attr && is_simple_identifier(c.name.as_str()) {
            self.pending_let_owner = Some(LetOwnerInfo {
                component_root: c.name.clone(),
                slot_name: SmolStr::new("default"),
            });
        }
    }

    fn visit_svelte_element(&mut self, s: &svn_parser::SvelteElement) {
        use svn_parser::SvelteElementKind;
        let ctx = WalkCtx {
            source: self.source,
        };
        // `<svelte:element this={dynamic}>` — tag only known at
        // runtime. Pass None so emit picks the generic HTMLElement
        // overload; actions that require a narrower base will
        // TS2345 against HTMLElement, matching user intent.
        walk_attributes(
            &s.attributes,
            &mut self.summary,
            &mut self.counters,
            &ctx,
            None,
        );
        // Reviewer item #1b: `<svelte:component this={X}>` and
        // `<svelte:self>` carry props / events / bindings just like
        // a regular `<Component>` instantiation. Route through the
        // same machinery with a synthetic `component_root` that emit
        // recognises:
        //   - SelfRef        → `__svn_self_default`
        //                       (resolves to the file's iso-component
        //                       interface via `__svn_create_component_any`)
        //   - Component      → `__svn_dyn_component[(<this expr>)]`
        //                       (unparseable raw — emit pulls out the
        //                       expression range and feeds it to
        //                       `__svn_ensure_component(EXPR)`)
        // Pre-fix these passed un-checked through a bare scope.
        match s.kind {
            SvelteElementKind::SelfRef => {
                collect_instantiation_inner(
                    SmolStr::from("__svn_self_default"),
                    &s.attributes,
                    &s.children,
                    s.range.start,
                    self.source,
                    &mut self.summary,
                );
            }
            SvelteElementKind::Component => {
                // Extract `this={X}`. The X expression text becomes
                // the `component_root` so emit's
                // `__svn_ensure_component(<root>)` resolves the
                // dynamic component value at the user's site. When
                // `this` is missing the directive degenerates to
                // `__svn_create_component_any`.
                let this_expr = s.attributes.iter().find_map(|a| {
                    let svn_parser::Attribute::Expression(e) = a else {
                        return None;
                    };
                    if e.name.as_str() != "this" {
                        return None;
                    }
                    self.source
                        .get(e.expression_range.start as usize..e.expression_range.end as usize)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .map(SmolStr::from)
                });
                let root = this_expr.unwrap_or_else(|| SmolStr::from("__svn_self_default"));
                // Filter out the `this={…}` directive itself from
                // the prop walk so it isn't surfaced as a regular
                // prop on the synthetic component.
                let attrs: Vec<svn_parser::Attribute> = s
                    .attributes
                    .iter()
                    .filter(|a| {
                        if let svn_parser::Attribute::Expression(e) = a {
                            e.name.as_str() != "this"
                        } else {
                            true
                        }
                    })
                    .cloned()
                    .collect();
                collect_instantiation_inner(
                    root,
                    &attrs,
                    &s.children,
                    s.range.start,
                    self.source,
                    &mut self.summary,
                );
            }
            _ => {}
        }
        // `bind:this` types differently across the `<svelte:*>` family:
        //
        //   - `<svelte:element>`        → DOM HTMLElement target (current).
        //   - `<svelte:component this>` → bound expr is a Component<…> ref.
        //   - `<svelte:self bind:this>` → bound expr is an instance of THIS component.
        //   - `<svelte:window/body/...>`→ no `bind:this` makes sense.
        //   - `<svelte:boundary>`       → no `bind:this`.
        //
        // The DOM-element check (`__svn_bind_this_check<HTMLElement>`)
        // wraps the bind expression with an HTMLElement-compatible
        // target type. Emitting it for the component-instance kinds
        // produces a wrong-shape diagnostic at the user's
        // `bind:this={x}` site (component instance fails
        // HTMLElement subtype check). Reviewer item #1a: gate the
        // collection to ONLY the `Element` kind. `Component`,
        // `SelfRef`, and `Boundary` `bind:this` get the proper
        // component-instance check from the full instantiation port
        // (#1b, deferred).
        if matches!(s.kind, SvelteElementKind::Element) {
            collect_bind_this_checks(&s.attributes, &mut self.summary);
        }
        // Bare `on:NAME` event-bubbling on `<svelte:body>` /
        // `<svelte:window>`. Each emits to a different DOM event-map
        // (`HTMLBodyElementEventMap` / `WindowEventMap`) so the
        // collector dispatches on the SvelteElementKind. Mirrors
        // upstream svelte2tsx `event-handler.ts:63-72` which routes
        // these through `__sveltets_2_mapBodyEvent` /
        // `__sveltets_2_mapWindowEvent`. `<svelte:document>` is
        // intentionally skipped — upstream's `event-handler.ts` doesn't
        // handle it either.
        match s.kind {
            SvelteElementKind::Body => collect_bubbled_dom_events(
                &s.attributes,
                BubbledDomEventScope::SvelteBody,
                &mut self.summary,
            ),
            SvelteElementKind::Window => collect_bubbled_dom_events(
                &s.attributes,
                BubbledDomEventScope::SvelteWindow,
                &mut self.summary,
            ),
            _ => {}
        }
    }

    fn visit_at_const(&mut self, bound_names: &[SmolStr], _expr_range: svn_core::Range) {
        // Push every bound name onto the shadow so subsequent
        // slot-attr / let-directive sites in the same fragment treat
        // them as scope-local. Destructure `{@const}` forms
        // (`{@const { a, b } = X}`) emit multiple names; bare
        // `{@const NAME = X}` emits one. The walker's fragment-level
        // bracket truncates them at exit.
        //
        // For the emit's `let NAME: any;` summary list, the legacy
        // shape is one name per `{@const}` (bare-identifier form
        // only). Destructure forms aren't currently surfaced in
        // `at_const_names` because emit doesn't yet declare per-
        // identifier `let` for them — that's tracked separately as
        // a follow-up. Until then, only push the FIRST name to the
        // summary list (matches pre-Phase-4 behaviour where
        // destructure forms were skipped entirely from the list).
        if let Some(first) = bound_names.first()
            && !is_destructure(bound_names)
            && self.counters.at_const_seen.insert(first.clone())
        {
            self.summary.at_const_names.push(first.clone());
        }
        for name in bound_names {
            // `{@const NAME = expr}` introduces a template-scope
            // binding without a value source we can rewrite (the
            // initialiser walks in the parent scope, but the bound
            // name itself is opaque to the slot resolver). Push as
            // `None` — bound but unresolvable. Slot-attr collection
            // drops references rather than splicing module-scope.
            self.shadow.entries.push((name.clone(), None));
        }
    }
}

/// Heuristic: a destructure `{@const}` produces multiple bound
/// names. Bare-identifier form produces exactly one. We use this to
/// gate `at_const_names` summary recording so destructure forms
/// don't leak partial names into the emit's `let NAME: any;` list
/// — emit can't currently synthesise multi-binding declarations
/// from the summary alone.
fn is_destructure(names: &[SmolStr]) -> bool {
    names.len() > 1
}

struct WalkCtx<'src> {
    source: &'src str,
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
    shadow: &ResolverStack,
    summary: &mut TemplateSummary,
) {
    use svn_parser::{AttrValuePart, Attribute as A};
    let mut slot_name = SmolStr::new("default");
    let mut entries: Vec<SlotAttr> = Vec::new();
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
            A::Plain(p) => {
                // Plain literal attrs on `<slot>` other than `name=`
                // (e.g. `<slot kind="header">`). Single-text-part
                // values flow through as TS string literals so
                // consumer-side `<Comp let:kind>` destructure resolves
                // `kind` to `"header"`. Multi-part interpolated
                // values (`<slot foo="a {b} c">`) and value-less
                // boolean shorthand are still skipped (full
                // SlotHandler port handles those).
                if let Some(v) = &p.value
                    && v.parts.len() == 1
                    && let AttrValuePart::Text { content, .. } = &v.parts[0]
                {
                    entries.push(SlotAttr::Prop {
                        name: p.name.clone(),
                        expr: SlotAttrExpr::Literal(content.to_string()),
                    });
                }
            }
            A::Expression(e) => {
                let start = e.expression_range.start as usize;
                let end = e.expression_range.end as usize;
                let Some(text) = source.get(start..end) else {
                    continue;
                };
                let trimmed = text.trim();
                // Resolver stack lookup for the expression's leading
                // identifier. Three states:
                //   - In stack with `Some(expr)` AND the expression
                //     is just a bare identifier — splice the resolved
                //     form. (Member / call / etc. fall to Stage 3's
                //     OXC rewriter — for now drop them.)
                //   - In stack with `None` — bound but unresolvable.
                //     Drop the slot attr; emitting at module scope
                //     would resolve to the wrong declaration.
                //   - Missing from stack — module-level identifier.
                //     Splice source verbatim.
                if let Some(head) = leading_identifier(trimmed)
                    && let Some(resolved) = shadow.lookup(head)
                {
                    if let Some(expr) = resolved {
                        // Only handle bare-identifier and shorthand-
                        // equivalent cases for now: `{item}` /
                        // `name={item}`. Member expressions
                        // (`{item.value}`) need OXC rewriting and
                        // stay dropped at this slice.
                        if trimmed == head {
                            entries.push(SlotAttr::Prop {
                                name: e.name.clone(),
                                expr: SlotAttrExpr::Resolved(expr.clone()),
                            });
                        }
                        // Non-bare expressions over a resolvable
                        // shadowed identifier — drop pending Stage 3.
                    }
                    // None: bound but unresolvable; drop.
                    continue;
                }
                entries.push(SlotAttr::Prop {
                    name: e.name.clone(),
                    expr: SlotAttrExpr::Range(e.expression_range),
                });
            }
            A::Shorthand(s) => {
                if let Some(resolved) = shadow.lookup(s.name.as_str()) {
                    if let Some(expr) = resolved {
                        entries.push(SlotAttr::Prop {
                            name: s.name.clone(),
                            expr: SlotAttrExpr::Resolved(expr.clone()),
                        });
                    }
                    // None or Some-but-non-bare-already-handled-above:
                    // shorthand always passes the bare-name check, so
                    // the only fall-through here is None (drop).
                    continue;
                }
                entries.push(SlotAttr::Prop {
                    name: s.name.clone(),
                    expr: SlotAttrExpr::Shorthand(s.name.clone()),
                });
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

/// Return the leading identifier of an expression source slice — the
/// run of identifier-valid bytes from the start, before any `.`,
/// `[`, `?.`, `(`, whitespace, or operator. For `item.id` returns
/// `"item"`; for `rest[0]` returns `"rest"`; for `user?.name` returns
/// `"user"`. Returns None when the slice doesn't start with an
/// identifier (e.g. `1 + foo`, `(x).y`).
///
/// Used by `collect_slot_def` to suppress slot-attr expressions whose
/// root binding is shadowed by an active template-scope let/each
/// binding — bare-identifier check alone misses member-access /
/// optional-chain / index-access shapes.
fn leading_identifier(s: &str) -> Option<&str> {
    let bytes = s.as_bytes();
    let first = *bytes.first()?;
    if !(first.is_ascii_alphabetic() || first == b'_' || first == b'$') {
        return None;
    }
    let mut end = 1;
    while end < bytes.len() {
        let b = bytes[end];
        if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' {
            end += 1;
        } else {
            break;
        }
    }
    Some(&s[..end])
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

/// SVELTE-4-COMPAT: collect bare `on:NAME` directives on a real DOM
/// element OR `<svelte:body>` / `<svelte:window>`. The bare form (no
/// `={handler}` value) is event-bubble shorthand — Svelte forwards
/// the native DOM event up to whichever ancestor listener fires for
/// the same name.
///
/// Emit projects each name into the event-map dictated by the
/// element scope:
///
///   - DOM element → `HTMLElementEventMap[NAME]`
///   - `<svelte:body>` → `HTMLBodyElementEventMap[NAME]`
///   - `<svelte:window>` → `WindowEventMap[NAME]`
///
/// so that consumers' `<Child on:click={cb}>` see the DOM event type
/// (`MouseEvent`, `KeyboardEvent`, …) rather than the lax
/// `CustomEvent<any>` fallback. Mirrors upstream svelte2tsx's
/// `__sveltets_2_mapElementEvent` / `__sveltets_2_mapBodyEvent` /
/// `__sveltets_2_mapWindowEvent` dispatch in `event-handler.ts:63-72`.
///
/// `<svelte:document>` is intentionally NOT routed here — upstream's
/// `event-handler.ts` doesn't handle it either. Component-bubbled
/// events (`<Child on:foo>` no value) are handled via
/// `TemplateSummary.has_bubbled_component_event`.
fn collect_bubbled_dom_events(
    attrs: &[Attribute],
    scope: BubbledDomEventScope,
    summary: &mut TemplateSummary,
) {
    for attr in attrs {
        let Attribute::Directive(d) = attr else {
            continue;
        };
        if d.kind != DirectiveKind::On {
            continue;
        }
        if d.value.is_some() {
            continue;
        }
        summary.bubbled_dom_events.push(BubbledDomEvent {
            name: d.name.clone(),
            scope,
            position: d.range.start,
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

/// Whether `s` is a valid bare JS identifier (no dots, no
/// brackets, no whitespace). Used to gate the SlotHandler
/// let-owner resolver — only simple-named components like
/// `<Wrapper let:foo>` get the `__SvnComponentSlots<typeof Wrapper>`
/// projection. Dotted forms (`<UI.Dropdown let:foo>`) would
/// produce malformed `typeof` references; those fall back to
/// the unresolved-shadow path.
fn is_simple_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    is_ident_start(first) && chars.all(is_ident_continue)
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
    collect_instantiation_inner(
        c.name.clone(),
        &c.attributes,
        &c.children,
        c.range.start,
        source,
        summary,
    );
}

/// Body of [`collect_component_instantiation`] generalised to accept
/// the parts directly. Reused by `<svelte:component this={X}>` and
/// `<svelte:self>` paths which don't have a `Component` AST node but
/// still want excess-prop / on-event / bind:this checking through
/// the same machinery. The `component_root` string is what emit
/// passes to `__svn_ensure_component(...)`; for synthetic kinds
/// (svelte:self → `__svn_self_default`, svelte:component →
/// `(EXPR)`), emit recognises the synthetic form and routes
/// accordingly.
fn collect_instantiation_inner(
    component_root: SmolStr,
    attributes: &[Attribute],
    children: &svn_parser::Fragment,
    range_start: u32,
    source: &str,
    summary: &mut TemplateSummary,
) {
    let mut props: Vec<PropShape> = Vec::with_capacity(attributes.len());
    let mut on_events: Vec<OnEventDirective> = Vec::new();
    let mut bind_this_target: Option<Range> = None;
    let mut component_bind_widen_targets: Vec<SmolStr> = Vec::new();
    // Detect "implicit children": any non-snippet, non-whitespace
    // child node between the open/close tags. Pure `{#snippet}`
    // children hoist as explicit props (different code path); pure
    // whitespace (formatting indent) is ignored.
    let has_implicit_children = children.nodes.iter().any(|n| match n {
        Node::SnippetBlock(_) => false,
        Node::Text(t) => !t.content.trim().is_empty(),
        _ => true,
    });
    for attr in attributes {
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
                // Multi-part interpolated attribute value
                // (`class="a {b} c"`) — emit as a TS template
                // literal `\`a ${b} c\`` so the embedded
                // expressions get type-checked AND the prop's
                // value carries a real string type. Mirrors upstream
                // svelte2tsx's `Attribute.ts:233`.
                props.push(PropShape::TemplateLiteral {
                    name: p.name.clone(),
                    parts: v.parts.clone(),
                    attr_range: p.range,
                });
                continue;
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
                    // `on:` prefix is 3 bytes; the name follows
                    // immediately (modifiers come after the name with
                    // `|` separators, stored in `d.modifiers`, NOT
                    // in `d.name`).
                    let name_start = d.range.start + 3;
                    let name_end = name_start + d.name.len() as u32;
                    let name_range = Range::new(name_start, name_end);
                    if let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                    {
                        on_events.push(OnEventDirective {
                            event_name: d.name.clone(),
                            name_range,
                            handler_range: *expression_range,
                        });
                    } else {
                        // `on:event` with no value — bare re-dispatch
                        // (event bubbling from sub-component).
                        //
                        // Reviewer follow-up #1: ALSO push an
                        // `OnEventDirective` with an empty range so
                        // emit produces `$inst.$on("event", () => {})`
                        // — type-checks the bubbled-event name against
                        // the child's declared Events surface. Pre-fix
                        // we only set `has_bubbled_component_event`
                        // and skipped the $on call entirely, so a
                        // bubbled event with a NAME the child doesn't
                        // declare passed silently. Mirrors upstream
                        // svelte2tsx's `EventHandler.ts:147` shape.
                        //
                        // The `has_bubbled_component_event` flag stays
                        // — it drives the OUTER component's
                        // default-export Props-widen (separate
                        // upstream behavior: components that
                        // re-dispatch sub-component events get their
                        // Props widened to `Record<string, any>` per
                        // upstream's `with_any_event` /
                        // `isomorphic_component` inference fallback).
                        on_events.push(OnEventDirective {
                            event_name: d.name.clone(),
                            name_range,
                            handler_range: Range::new(d.range.start, d.range.start),
                        });
                        summary.has_bubbled_component_event = true;
                        // Reviewer follow-up #2: also record the
                        // (event_name, component_root) pair so the
                        // wrapper's own `$$Events` surface carries the
                        // bubbled name. Emit projects via
                        // `__SvnComponentEvents<typeof <root>>["NAME"]`
                        // and intersects with `events_alias_body`.
                        summary
                            .bubbled_component_events
                            .push(BubbledComponentEvent {
                                event_name: d.name.clone(),
                                component_root: component_root.clone(),
                                position: d.range.start,
                            });
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
                        | PropShape::GetSetBinding { name, .. }
                        | PropShape::TemplateLiteral { name, .. } => name != &target,
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
                        | PropShape::GetSetBinding { name, .. }
                        | PropShape::TemplateLiteral { name, .. } => name != &target,
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
                        | PropShape::GetSetBinding { name, .. }
                        | PropShape::TemplateLiteral { name, .. } => name != &target,
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
            component_root,
            props,
            has_implicit_children,
            on_events,
            bind_this_target,
            component_bind_widen_targets,
            node_start: range_start,
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
    fn no_template_check_void_registration() {
        // Post-Gap-C: template-check is an arrow-expression statement
        // (no name to forward-reference), so the analyze walker no
        // longer pre-registers `__svn_tpl_check` in void_refs.
        let s = walk_str("<p>hi</p>");
        assert!(!s.void_refs.contains("__svn_tpl_check"));
    }

    #[test]
    fn use_directive_registers_action_attrs() {
        // Each `use:foo` directive needs an `__svn_action_attrs_N` holder
        // declared in the template-check function so its inferred attribute
        // type doesn't go unused.
        let s = walk_str(r#"<div use:tooltip={{ text: 'hi' }}>x</div>"#);
        assert!(s.void_refs.contains("__svn_action_attrs_0"));
    }

    #[test]
    fn multiple_use_directives_get_unique_indices() {
        let s = walk_str(r#"<div use:a use:b><span use:c /></div>"#);
        assert!(s.void_refs.contains("__svn_action_attrs_0"));
        assert!(s.void_refs.contains("__svn_action_attrs_1"));
        assert!(s.void_refs.contains("__svn_action_attrs_2"));
    }

    #[test]
    fn bind_pair_registers_bind_pair() {
        // `bind:foo={getter, setter}` declares a tuple holder; without a
        // void-reference, TypeScript flags it as unused.
        let s = walk_str("<input bind:value={() => g(), (v) => s(v)} />");
        assert!(s.void_refs.contains("__svn_bind_pair_0"));
    }

    #[test]
    fn simple_bind_does_not_register_bind_pair() {
        let s = walk_str("<input bind:value={x} />");
        assert!(
            !s.void_refs
                .names()
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
        assert!(s.void_refs.contains("__svn_action_attrs_0"));
    }

    #[test]
    fn directives_in_block_body_are_walked() {
        let s = walk_str("{#if cond}<div use:focus />{/if}");
        assert!(s.void_refs.contains("__svn_action_attrs_0"));
    }

    #[test]
    fn each_alternate_branch_walked() {
        let s = walk_str("{#each items as i}<x />{:else}<div use:focus />{/each}");
        assert!(s.void_refs.contains("__svn_action_attrs_0"));
    }
}
