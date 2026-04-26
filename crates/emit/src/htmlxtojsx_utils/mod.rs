//! Filename-parity stubs for upstream
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/utils/`.
//!
//! Upstream's utility module exports byte-range manipulation helpers
//! (`surroundWith`, `getDirectiveNameStartEndIdx`,
//! `withTrailingPropertyAccess`, `transform`, `TransformationArray`)
//! used by every per-node-type emitter in `htmlxtojsx_v2/nodes/`. Most
//! are MagicString-driven — applicable to upstream's text-rewrite
//! emit but NOT to our structural emit (we never overwrite source
//! bytes; we build a new overlay from scratch).
//!
//! This directory exists as a navigation aid for contributors familiar
//! with upstream's layout: `htmlxtojsx_v2/utils/<X>.ts` lands here at
//! `htmlxtojsx_utils/<x>.rs`.

pub(crate) mod node_utils;
