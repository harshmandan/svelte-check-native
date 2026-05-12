//! `<svelte:element>` analyze pass — no direct upstream equivalent
//! (Element.ts handles both static and dynamic).

use smol_str::SmolStr;
use svn_parser::{SvelteElement, SvelteElementKind};

use crate::nodes::attribute::{WalkCtx, walk_attributes};
use crate::nodes::binding::collect_bind_this_checks;
use crate::nodes::event_handler::collect_bubbled_dom_events;
use crate::nodes::inline_component::collect_instantiation_inner;
use crate::walker::{AnalyzeVisitor, BubbledDomEventScope};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, s: &SvelteElement) {
    let ctx = WalkCtx { source: v.source };
    // `<svelte:element this={dynamic}>` — tag only known at
    // runtime. Pass None so emit picks the generic HTMLElement
    // overload; actions that require a narrower base will
    // TS2345 against HTMLElement, matching user intent.
    walk_attributes(&s.attributes, &mut v.summary, &mut v.counters, &ctx, None);
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
                v.source,
                &mut v.summary,
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
                v.source
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
                v.source,
                &mut v.summary,
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
        collect_bind_this_checks(&s.attributes, &mut v.summary);
    }
    // Bare `on:NAME` event-bubbling on `<svelte:body>` /
    // `<svelte:window>` / `<svelte:element>`. Each emits to a
    // different DOM event-map (`HTMLBodyElementEventMap` /
    // `WindowEventMap` / `HTMLElementEventMap`) so the collector
    // dispatches on the SvelteElementKind. Mirrors upstream
    // svelte2tsx `event-handler.ts:63-72` which routes these
    // through `__sveltets_2_mapBodyEvent` /
    // `__sveltets_2_mapWindowEvent` / `__sveltets_2_mapElementEvent`.
    // `<svelte:document>` is intentionally skipped — upstream's
    // `event-handler.ts` doesn't handle it either.
    //
    // `<svelte:element>` reuses the regular-element scope
    // (`HTMLElementEventMap`) — the dynamic-tag form picks an
    // arbitrary HTML element at runtime, so the broadest DOM-event
    // map matches what consumers see when bubbling. Closes c4
    // false-positive on `<Button on:click={onSubmit}>` where Button
    // forwards from `<svelte:element on:click>` and pre-fix the
    // bubble was missed entirely (events fell through to
    // `{[k:string]: CustomEvent<any>}` index sig, making
    // `(e?: MouseEvent) => void` handlers fail TS2345 against the
    // resolved `(e: CustomEvent<any>) => void` callback).
    match s.kind {
        SvelteElementKind::Body => collect_bubbled_dom_events(
            &s.attributes,
            BubbledDomEventScope::SvelteBody,
            &mut v.summary,
        ),
        SvelteElementKind::Window => collect_bubbled_dom_events(
            &s.attributes,
            BubbledDomEventScope::SvelteWindow,
            &mut v.summary,
        ),
        SvelteElementKind::Element => collect_bubbled_dom_events(
            &s.attributes,
            BubbledDomEventScope::Element,
            &mut v.summary,
        ),
        _ => {}
    }
}
