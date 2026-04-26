//! Static list of known DOM event-handler attribute names.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/knownevents.ts`.
//!
//! **Status: not needed by our architecture.**
//!
//! Upstream uses this list to decide whether an `oncamelCase` attribute
//! is a known DOM event handler (vs. a custom prop). The check is part
//! of attribute-classification: if the attribute name appears in
//! knownevents, it routes through the on-event handler path; otherwise
//! it's treated as a regular attribute.
//!
//! We don't need this distinction — our DOM-element emit (see
//! [`crate::nodes::element::emit_dom_element_open`]) emits ALL
//! attributes through the `svelteHTML.createElement(tag, { …attrs })`
//! literal, and tsgo's strict-typing of `HTMLElementTagNameMap[tag]`
//! validates the attribute against the typed event-handler slot
//! directly. There's no separate "is this a known event?" branch in
//! our code; the type system answers it.
//!
//! `on:event` directives (the Svelte source-syntax `on:click={handler}`)
//! are a different concern and are handled by
//! [`crate::nodes::event_handler`] (DOM path) and
//! [`crate::nodes::inline_component::emit_on_event_calls`] (component
//! path).
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `knownevents.ts` should land here and understand why we
//! don't need the equivalent data.
