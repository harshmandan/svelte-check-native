//! Rule modules — one per warning family.
//!
//! Each sub-module exposes a `visit(node, ctx)` entry point called
//! from `walk::walk_fragment`.

pub mod a11y_rules;
pub mod binding_rules;
pub mod block_rules;
pub mod component_rules;
pub mod element_rules;
pub mod implicit_close;
pub mod script_ast_rules;
pub mod script_rules;
pub mod svelte_element_rules;
pub mod text_rules;

// Utility helpers shared by rule modules.
pub mod util;
