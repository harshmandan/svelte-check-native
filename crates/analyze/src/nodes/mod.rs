//! Per-node analyze passes — one file per upstream `htmlxtojsx_v2`
//! node kind, matching the emit-side `crates/emit/src/nodes/` layout
//! file-for-file so a developer can navigate either side by basename.
//!
//! Most node arms have migrated here from `template_walker.rs`;
//! `animation.rs`, `if_else_block.rs`, and `transition.rs` are still
//! stubs whose arms remain in `template_walker.rs`.

pub mod action;
pub mod animation;
pub mod attribute;
pub mod await_pending_catch_block;
pub mod binding;
pub mod const_tag;
pub mod destructure;
pub mod each_block;
pub mod element;
pub mod event_handler;
pub mod if_else_block;
pub mod inline_component;
pub mod let_directive;
pub mod snippet_block;
pub mod svelte_element;
pub mod transition;
