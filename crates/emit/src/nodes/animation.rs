//! `animate:NAME(PARAMS)` animation directive.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Animation.ts`.
//!
//! Upstream emits a typed call:
//!
//! ```text
//!     __sveltets_2_ensureAnimation(
//!         NAME(svelte.mapElementTag('tag'), __sveltets_2_AnimationMove, (PARAMS))
//!     );
//! ```
//!
//! so tsgo type-checks PARAMS against NAME's declared third-parameter
//! shape and validates NAME's return against `AnimationConfig`.
//!
//! **Status: feature gap.** We don't recognise `animate:` directives.
//! `<div animate:flip={...}>` produces no animation type-check;
//! `flip(...)`'s parameter contract is unverified.
//!
//! Implementation when landed: extend
//! `nodes/element.rs::emit_dom_element_open`'s directive scan to dispatch
//! `DirectiveKind::Animation` to a handler here, mirroring upstream's
//! `__sveltets_2_ensureAnimation` shape via our `__svn_ensure_animation`
//! ambient.
//!
//! This file exists for parity navigation.
