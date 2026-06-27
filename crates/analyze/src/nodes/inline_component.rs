//! `<Component>` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/InlineComponent.ts`.

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{AttrValuePart, Attribute, Component, Node};

use crate::nodes::attribute::{WalkCtx, literal_attr_value, walk_attributes};
use crate::nodes::destructure::{is_simple_identifier, simple_identifier_in};
use crate::walker::{
    AnalyzeVisitor, BindDirective, BubbledComponentEvent, ComponentInstantiation, LetOwnerInfo,
    OnEventDirective, PropShape, TemplateSummary,
};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, c: &Component) {
    let ctx = WalkCtx { source: v.source };
    // `use:` on a component is nonsensical at the Svelte level
    // (actions attach to DOM elements, not to component instances),
    // but we pass it along — emit's shim-side
    // `__svn_map_element_tag(tag: string)` overload resolves
    // unknown tags to `HTMLElement` so the pattern doesn't break
    // the program.
    walk_attributes(&c.attributes, &mut v.summary, &mut v.counters, &ctx, None);
    collect_component_instantiation(c, v.source, &mut v.summary);
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
        v.pending_let_owner = Some(LetOwnerInfo {
            component_root: c.name.clone(),
            slot_name: SmolStr::new("default"),
        });
    }
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
pub(crate) fn collect_component_instantiation(
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
pub(crate) fn collect_instantiation_inner(
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
    let mut bind_directives: Vec<BindDirective> = Vec::new();
    // Detect "implicit children": any non-snippet, non-whitespace
    // child node between the open/close tags. Pure `{#snippet}`
    // children hoist as explicit props (different code path); pure
    // whitespace (formatting indent) is ignored.
    //
    // Skip when the component carries any `let:NAME` directive: the
    // body content is then a slot-let scope, NOT a `children` prop
    // surface. Emitting `children: () => __svn_snippet_return()`
    // against a component declaring `Record<string, never>` props
    // (no children, no slots either) fires TS2322 spuriously, where
    // upstream silently routes the body through the slot scope.
    // Example: `<Comp let:b>...</Comp>` against a Comp with no
    // declared `children: Snippet`.
    let has_let_directive = attributes
        .iter()
        .any(|a| matches!(a, Attribute::Directive(d) if d.kind == svn_parser::DirectiveKind::Let));
    let has_implicit_children = !has_let_directive
        && children.nodes.iter().any(|n| match n {
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
                // R-Conv #19 (D-ii fix #3): anchor 2353 / 2322 on the
                // NAME (`o` of `{only_bind}`), not the opening brace.
                // Upstream LS's reverse-map for `<Foo {only_bind} />`
                // points at the prop name's first byte — matches
                // `bindings` fixture line 28 col 8 vs ours pre-fix
                // col 7.
                let inner = source
                    .get(s.range.start as usize + 1..s.range.end as usize)
                    .unwrap_or("");
                let leading_ws = (inner.len() - inner.trim_start().len()) as u32;
                let name_start = s.range.start + 1 + leading_ws;
                let name_end = name_start + s.name.len() as u32;
                props.push(PropShape::Shorthand {
                    name: s.name.clone(),
                    attr_range: svn_core::Range::new(name_start, name_end),
                });
            }
            Attribute::Comment(_) => {}
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
                        // Reviewer follow-up #1: push an
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
                        // Round-7 follow-up #7: upstream's
                        // `event-handler.ts:12-15` skips
                        // `handleEventHandlerBubble` when the parent
                        // is `<svelte:self>` — bubbling self's own
                        // events into the wrapper's `$$Events` is a
                        // no-op (the parent component IS the child)
                        // and would wrongly disqualify the runes
                        // fn_component shape and trigger the
                        // Svelte-4 props-widen path. The `$inst.$on`
                        // call still fires above so the bubbled name
                        // type-checks against self's events surface;
                        // we just don't register a bubble for it.
                        if component_root.as_str() == "__svn_self_default" {
                            continue;
                        }
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
                // The `bind:NAME={getter, setter}` get/set form
                // (`BindPair`) is handled by the dedicated branch
                // below — it never reaches this `bind:this` arm.
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
                    props.retain(|p| p.name() != Some(&target));
                    // R-Conv #1: anchor diagnostics on the property
                    // NAME (`prop` in `bind:prop={…}`), not the
                    // whole `bind:prop={…}` slice. Upstream's LS
                    // reverse-mapping for component-bind sites
                    // points at the name (`$store-bind` fixture's
                    // expected col 16 = start of `prop`, not col 11
                    // = start of `bind:`). Using `d.range` here
                    // anchored 5 chars too early.
                    let name_start = d.range.start + d.kind.prefix_len_with_colon();
                    let name_end = name_start + target.len() as u32;
                    let name_range = svn_core::Range::new(name_start, name_end);
                    props.push(PropShape::Expression {
                        name: target,
                        expr_range: *expression_range,
                        attr_range: name_range,
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
                    // R-Conv #19 (D-ii fix #4): record the prop NAME +
                    // `bind:NAME` source range so emit can write
                    // `__svn_inst_N.$$bindings = 'NAME';` post-instance
                    // for the literal-Bindings union check.
                    bind_directives.push(BindDirective {
                        name: d.name.clone(),
                        range: d.range,
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
                    props.retain(|p| p.name() != Some(&target));
                    // Bare `bind:NAME` is `bind:NAME={NAME}` — same
                    // widening trailer as the explicit form.
                    component_bind_widen_targets.push(target.clone());
                    // R-Conv #19 (D-ii fix #3): anchor diagnostics on
                    // the NAME, not the `bind:` prefix. Mirrors the
                    // explicit-expression arm at line 2324 — upstream
                    // LS's reverse-map points at the prop name (e.g.
                    // `bindings` fixture line 27 col 12 = `o` of
                    // `only_bind`, not col 7 = `b` of `bind:`).
                    let name_start = d.range.start + d.kind.prefix_len_with_colon();
                    let name_end = name_start + target.len() as u32;
                    let name_range = svn_core::Range::new(name_start, name_end);
                    props.push(PropShape::Shorthand {
                        name: target.clone(),
                        attr_range: name_range,
                    });
                    bind_directives.push(BindDirective {
                        name: target,
                        range: d.range,
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
                    props.retain(|p| p.name() != Some(&target));
                    props.push(PropShape::GetSetBinding {
                        name: target.clone(),
                        getter_range: *getter_range,
                        setter_range: *setter_range,
                        attr_range: d.range,
                    });
                    bind_directives.push(BindDirective {
                        name: target,
                        range: d.range,
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
            bind_directives,
            node_start: range_start,
        });
}
