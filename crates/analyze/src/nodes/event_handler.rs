//! `on:` directive analyze pass — mirrors upstream
//! `svelte2tsx/nodes/event-handler.ts` (bubble collection + element
//! event-map projection); the no-value bubble branch is also detected
//! by `htmlxtojsx_v2/nodes/EventHandler.ts`.

use svn_parser::{Attribute, DirectiveKind};

use crate::walker::{BubbledDomEvent, BubbledDomEventScope, TemplateSummary};

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
/// `<svelte:document>` is intentionally NOT routed here. Upstream's
/// `event-handler.ts` *does* register a bare handler on it, but its
/// `getEventDefExpressionForNonComponent` switch has no `Document`
/// case, so it falls through `default` and maps the event to
/// `undefined` (a useless typing). We omit it deliberately: bare event
/// forwarding on `<svelte:document>` is an exotic-to-invalid edge, and
/// our `CustomEvent<any>` fallback is no worse in practice.
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
