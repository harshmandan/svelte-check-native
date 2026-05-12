//! `on:` directive analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/EventHandler.ts`.

use svn_parser::{Attribute, DirectiveKind};

use crate::template_walker::{BubbledDomEvent, BubbledDomEventScope, TemplateSummary};

/// SVELTE-4-COMPAT: collect bare `on:NAME` directives on a real DOM
/// element OR `<svelte:body>` / `<svelte:window>`. The bare form (no
/// `={handler}` value) is event-bubble shorthand — Svelte forwards
/// the native DOM event up to whichever ancestor listener fires for
/// the same name.
///
/// Emit projects each name into the event-map dictated by the
/// element scope, following upstream svelte2tsx's swapped naming
/// convention (the upstream helpers' map types are inverted relative
/// to their names, and we mirror the actual map types byte-for-byte
/// so consumer-side event types match):
///
///   - DOM element → `HTMLElementEventMap[NAME]`
///   - `<svelte:body>` → `WindowEventMap[NAME]`
///   - `<svelte:window>` → `HTMLBodyElementEventMap[NAME]`
///
/// so that consumers' `<Child on:click={cb}>` see the DOM event type
/// (`MouseEvent`, `KeyboardEvent`, …) rather than the lax
/// `CustomEvent<any>` fallback. Mirrors upstream svelte2tsx's
/// `__sveltets_2_mapElementEvent` / `__sveltets_2_mapBodyEvent` /
/// `__sveltets_2_mapWindowEvent` dispatch in `event-handler.ts:63-72`,
/// with the same swapped K-type bound (`mapBodyEvent<K extends keyof
/// WindowEventMap>` / `mapWindowEvent<K extends keyof
/// HTMLBodyElementEventMap>` per `svelte-shims.d.ts:185-190`).
///
/// `<svelte:document>` is intentionally NOT routed here — upstream's
/// `event-handler.ts` doesn't handle it either. Component-bubbled
/// events (`<Child on:foo>` no value) are handled via
/// `TemplateSummary.has_bubbled_component_event`.
pub(crate) fn collect_bubbled_dom_events(
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
