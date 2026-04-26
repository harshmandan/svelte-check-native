//! Per-node-type emitters.
//!
//! Each submodule owns the emit logic for one Svelte AST node category,
//! mirroring upstream svelte2tsx's `htmlxtojsx_v2/nodes/` layout. The
//! main dispatcher lives in `lib.rs::emit_template_node` and forwards
//! per-node-type to the helpers exposed here.

pub(crate) mod await_pending_catch_block;
pub(crate) mod component;
pub(crate) mod const_tag;
pub(crate) mod directives;
pub(crate) mod each_block;
pub(crate) mod element;
pub(crate) mod if_else_block;
pub(crate) mod mustache_tag;
pub(crate) mod snippet_block;
