//! Component event-shape extraction (`$$Events` interface, typed
//! `createEventDispatcher<T>()`, `on:event` directive collection).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ComponentEvents.ts`.
//!
//! **Status: split across analyze + emit layers.**
//!
//! Upstream's `ComponentEvents` is a stateful class that:
//!
//! 1. Detects whether the component has a `$$Events` interface or a
//!    typed `createEventDispatcher<T>()` call.
//! 2. Tracks each `on:NAME` directive consumed by parents.
//! 3. Builds an "events" surface (the `Events` slot of
//!    `SvelteComponent<P, E, S>`) used by the default-export shape.
//!
//! Our equivalent splits these concerns:
//!
//! - **Detection** of `$$Events` and the `<script strictEvents>`
//!   opt-in lives in [`crate::svelte4::compat::has_strict_events`] and
//!   [`crate::svelte4::compat::has_strict_events_attr`]. The third
//!   trigger (Svelte 5 runes mode) is detected by
//!   [`crate::svelte4::compat::is_runes_mode`].
//! - **Per-component event collection** (the `on:NAME={handler}`
//!   directives consumed at instantiation sites) lives in
//!   `analyze::template_walker` as `ComponentInstantiation::on_events`.
//! - **Emit** of the `$inst.$on("NAME", (handler))` calls is in
//!   [`crate::nodes::inline_component::emit_on_event_calls`].
//! - **Default-export Events surface** (`{ [K in keyof $$Events]:
//!   CustomEvent<$$Events[K]> }` mapped type or the lax
//!   `{ [evt: string]: CustomEvent<any> }` fallback) is built in
//!   [`crate::render_function::emit_render_body_return`].
//!
//! No typed-`createEventDispatcher<T>()` extraction: we don't yet
//! parse the Svelte-5-style typed dispatcher's generic argument back
//! into a `$$Events`-shaped projection. A child using the typed
//! dispatcher without a `$$Events` interface routes through the lax
//! `[evt: string]: CustomEvent<any>` overload (matches v0.2.5 behaviour;
//! tracked as a lint-only follow-up in `notes/`).
//!
//! This file exists for parity navigation.
