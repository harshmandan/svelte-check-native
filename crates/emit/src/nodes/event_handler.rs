//! `on:event={handler}` event-handler directive.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/EventHandler.ts`.
//!
//! Upstream's `handleEventHandler` dispatches on whether the parent is
//! an `Element` (DOM) or `InlineComponent`:
//!
//! - **DOM element** — emit as a `"on:NAME": (handler)` attribute on
//!   the `createElement` literal. tsgo checks the handler against the
//!   element's typed event map (`HTMLElementEventMap['NAME']`).
//! - **Component** — emit as `inst.$on("NAME", (handler))` after the
//!   `new` call. tsgo checks the handler against the component's
//!   declared `Events` record via `SvelteComponent<P, E, S>.$on`.
//!
//! **Our split:** the same dual-dispatch lives across two existing
//! modules in our tree:
//!
//! - DOM `on:event` is handled inline by `lib.rs::emit_template_node`
//!   when walking `Node::Element`'s attribute list. The directive is
//!   converted to an `onNAME` prop key during attribute emission in
//!   `nodes/element.rs::emit_dom_element_open`.
//! - Component `on:event` is handled by
//!   `nodes/inline_component.rs::emit_on_event_calls`, which emits one
//!   `$inst.$on(...)` line per directive after the component's `new`
//!   expression.
//!
//! Consolidating into a single `event_handler.rs` with a dispatch
//! parameter would mirror upstream more closely, but the DOM path is
//! tangled with the rest of `emit_dom_element_open`'s attribute loop
//! (it shares state with the createElement literal builder), and the
//! component path is tangled with the post-`new` trailers
//! (`emit_component_bind_widen_trailers`, `emit_bind_this_assignment`).
//! Splitting would require exposing intermediate state across module
//! boundaries with no real payoff. The current split mirrors how the
//! emit actually flows.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `EventHandler.ts` should land here and find the
//! pointers to the actual call sites.
