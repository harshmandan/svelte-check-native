//! `<svelte:element>` analyze pass — no direct upstream equivalent
//! (Element.ts handles both static and dynamic).

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{SvelteElement, SvelteElementKind};

use crate::nodes::attribute::{WalkCtx, walk_attributes};
use crate::nodes::event_handler::collect_bubbled_dom_events;
use crate::nodes::inline_component::{InstantiationRoot, collect_instantiation_inner};
use crate::walker::{AnalyzeVisitor, BubbledDomEventScope, ComponentCtor};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, s: &SvelteElement) {
    let ctx = WalkCtx { source: v.source };
    // `<svelte:element this={dynamic}>` — tag only known at
    // runtime. Pass None so emit picks the generic HTMLElement
    // overload; actions that require a narrower base will
    // TS2345 against HTMLElement, matching user intent.
    walk_attributes(&s.attributes, &mut v.summary, &mut v.counters, &ctx, None);
    // `<svelte:component this={X}>` and `<svelte:self>` carry
    // props / events / bindings just like a regular `<Component>`
    // instantiation, so route them through the same machinery with
    // the constructor svelte2tsx uses for each:
    //   - SelfRef        → untyped `createComponentAny({ props })`
    //   - Component      → `__svn_ensure_component(<this expr>)`
    // Pre-fix these passed un-checked through a bare scope.
    if matches!(
        s.kind,
        SvelteElementKind::SelfRef | SvelteElementKind::Component
    ) {
        crate::nodes::let_directive::enter_component_like(
            v,
            s.range.start,
            &s.attributes,
            &s.children,
            None,
        );
    } else {
        crate::nodes::let_directive::enter_element_like(v, s.range.start, &s.attributes);
    }
    match s.kind {
        SvelteElementKind::SelfRef => {
            collect_instantiation_inner(
                self_root(s.range.start),
                &s.attributes,
                &s.children,
                s.range,
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
                let raw = v
                    .source
                    .get(e.expression_range.start as usize..e.expression_range.end as usize)?;
                let text = raw.trim();
                if text.is_empty() {
                    return None;
                }
                let start = e.expression_range.start + (raw.len() - raw.trim_start().len()) as u32;
                let end = start + text.len() as u32;
                Some(InstantiationRoot {
                    ctor: ComponentCtor::Ensured {
                        root: SmolStr::from(text),
                        range: Range::new(start, end),
                    },
                    // The constructor reference follows the moved
                    // expression, where upstream's map resolves it to
                    // the character right after it.
                    ctor_anchor: Range::new(end, end + 1),
                })
            });
            let root = this_expr.unwrap_or_else(|| self_root(s.range.start));
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
                s.range,
                v.source,
                &mut v.summary,
            );
        }
        _ => {}
    }
    // Bare `on:NAME` event-bubbling on `<svelte:body>` /
    // `<svelte:window>` / `<svelte:element>`. Each emits to a
    // different DOM event-map (`HTMLBodyElementEventMap` /
    // `WindowEventMap` / `HTMLElementEventMap`) so the collector
    // dispatches on the SvelteElementKind. Mirrors upstream
    // svelte2tsx `event-handler.ts:63-72` which routes these
    // through `__sveltets_2_mapBodyEvent` /
    // `__sveltets_2_mapWindowEvent` / `__sveltets_2_mapElementEvent`.
    // `<svelte:document>` is intentionally omitted. Upstream's
    // `event-handler.ts` has no dedicated Document arm, so its
    // bubbled `on:NAME` falls to
    // `getEventDefExpressionForNonComponent`'s default and the
    // name is registered with an `undefined`-valued events entry.
    // We skip it because that resolved type is `undefined` and the
    // practical impact is immaterial; if exact surface parity is
    // ever required, register the name with an `undefined` value
    // (a no-payload variant) rather than a DocumentEventMap lookup,
    // which would make us MORE specific than upstream.
    //
    // `<svelte:element>` reuses the regular-element scope
    // (`HTMLElementEventMap`) — the dynamic-tag form picks an
    // arbitrary HTML element at runtime, so the broadest DOM-event
    // map matches what consumers see when bubbling. Without this,
    // a `<Button on:click={onSubmit}>` that forwards from
    // `<svelte:element on:click>` loses the bubble — events fall
    // through to the `{[k:string]: CustomEvent<any>}` index
    // signature, making `(e?: MouseEvent) => void` handlers fail
    // TS2345 against the resolved `(e: CustomEvent<any>) => void`
    // callback.
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
        SvelteElementKind::Element => {
            collect_bubbled_dom_events(&s.attributes, BubbledDomEventScope::Element, &mut v.summary)
        }
        _ => {}
    }
}

/// Root of a `<svelte:self>` (or a `<svelte:component>` without `this`):
/// an untyped instance, anchored on the tag name.
fn self_root(node_start: u32) -> InstantiationRoot {
    let start = node_start + 1;
    InstantiationRoot {
        ctor: ComponentCtor::SelfAny,
        ctor_anchor: Range::new(start, start + 1),
    }
}
