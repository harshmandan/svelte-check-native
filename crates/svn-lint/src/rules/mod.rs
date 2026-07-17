//! Rule modules — one per warning family.
//!
//! Template-fragment rules are driven from `walk::walk_fragment_impl`:
//! `element_rules`, `component_rules`, and `svelte_element_rules`
//! expose `visit(...)`; `block_rules` exposes `visit_if`/`visit_each`/
//! `visit_key`/`visit_await`; `text_rules` exposes `visit_text`; and
//! `a11y_rules` (`visit_regular`/`visit_dynamic`) is invoked from the
//! element/svelte-element visitors. `script_rules` exposes
//! `visit_document(doc, ctx)`; `script_ast_rules` hooks into the
//! scope builder's shared script walk and exposes `flush(events,
//! ctx)`; `implicit_close` exposes `scan(source, ctx)`, all driven
//! from `walk::walk`. `binding_rules::visit(ctx)` also runs from
//! `walk::walk`.

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
