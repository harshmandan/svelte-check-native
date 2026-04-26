//! `transition:NAME(PARAMS)` / `in:NAME(PARAMS)` / `out:NAME(PARAMS)`
//! directives.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Transition.ts`.
//!
//! Upstream emits a typed call:
//!
//! ```text
//!     __sveltets_2_ensureTransition(
//!         NAME(svelte.mapElementTag('tag'), (PARAMS))
//!     );
//! ```
//!
//! so tsgo type-checks PARAMS against NAME's declared second-parameter
//! shape and validates NAME's return against `TransitionConfig | (() =>
//! TransitionConfig)`.
//!
//! **Status: feature gap.** We don't recognise `transition:` / `in:` /
//! `out:` directives. `<div transition:fade={...}>` produces no
//! transition type-check; `fade(...)`'s parameter contract is
//! unverified.
//!
//! Implementation when landed: extend
//! `nodes/element.rs::emit_dom_element_open`'s directive scan to dispatch
//! `DirectiveKind::Transition` / `In` / `Out` to a handler here,
//! mirroring upstream's `__sveltets_2_ensureTransition` shape via our
//! `__svn_ensure_transition` ambient.
//!
//! This file exists for parity navigation.
