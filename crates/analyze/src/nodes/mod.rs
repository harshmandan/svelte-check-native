//! Per-node analyze passes — one file per upstream `htmlxtojsx_v2`
//! node kind, matching the emit-side `crates/emit/src/nodes/` layout
//! file-for-file so a developer can navigate either side by basename.
//!
//! Only node kinds with analyze-phase work appear here. Kinds whose
//! work is purely emit-side (`{#if}`/`{:else}`, `transition:`/`in:`/
//! `out:`, `animate:`) have no module here; their scope handling lives
//! in `walker.rs` / `template_scope.rs` and their ref collection in
//! `template_refs.rs`.

pub mod action;
pub mod attribute;
pub mod await_pending_catch_block;
pub mod binding;
pub mod const_tag;
pub mod destructure;
pub mod each_block;
pub mod element;
pub mod event_handler;
pub mod inline_component;
pub mod let_directive;
pub mod snippet_block;
pub mod svelte_element;
