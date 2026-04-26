//! Per-node-type emitters.
//!
//! Each submodule owns the emit logic for one Svelte AST node category,
//! mirroring upstream svelte2tsx's `htmlxtojsx_v2/nodes/` layout. The
//! main dispatcher lives in `lib.rs::emit_template_node` and forwards
//! per-node-type to the helpers exposed here.

pub(crate) mod blocks;
pub(crate) mod component;
pub(crate) mod element;
pub(crate) mod interpolation;
