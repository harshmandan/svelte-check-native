//! Semantic analysis passes over the Svelte AST.
//!
//! Two product-type outputs are bundled into [`SemanticModel`]:
//!
//! - [`PropsInfo`] — Props-shape decision: type text, root name,
//!   destructured locals, source kind. Built once per file from the
//!   parsed instance script.
//! - [`TemplateSummary`] — output of the structural template walk:
//!   bind:this targets, `{@const}` names, `<slot>` definitions,
//!   component instantiations, action directives, and the
//!   [`VoidRefRegistry`] of synthesized names emit must reference.
//!
//! Centralizing the void-ref registry is what stops emit from
//! having to remember a per-feature `void <name>;` line every time
//! a new emission landed. Every kind of synthesized name
//! (template-check wrapper, action attrs, bind pairs, store
//! aliases, prop locals) registers there; emit reads the registry
//! once and writes a single consolidated `void (...)` block.
//!
//! In addition to the bundled outputs, the crate exports stateful
//! accumulator helpers — `collect_top_level_bindings`,
//! `find_store_refs_with_bindings`, `find_template_refs`,
//! `collect_typed_uninit_lets`, `collect_typed_top_level_lets`.
//! These are driven by emit at specific points in its flow (e.g.
//! `collect_top_level_bindings` is called three times to union
//! identifiers from module + instance + rewritten-instance
//! programs). They join `SemanticModel` only when a second consumer
//! needs the same accumulated set up-front — see `CLAUDE.md`'s
//! "don't invent placeholder fields with no reader" rule.
//!
//! All passes share a single `Visitor` walk of the AST. One pass,
//! many collectors.

// Tests are allowed to panic loudly on setup failures.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod dom_binding;
pub mod jsdoc;
pub mod model;
pub mod props;
pub mod rune;
pub mod store;
pub mod template_refs;
pub mod template_scope;
pub mod template_walker;
pub mod void_refs;

pub use jsdoc::{
    scan_jsdoc_props_typedef_keys, scan_jsdoc_typedef_name, should_synthesise_js_props,
};
pub use model::SemanticModel;
pub use props::{
    PropInfo, PropsInfo, PropsSource, contains_typeof_ref, find_dispatched_event_names,
    find_dispatcher_event_type_source, find_dispatcher_local_names, has_event_dispatcher_call,
    root_type_name_of,
};
pub use rune::{RuneCall, RuneKind, find_runes};
pub use store::{
    collect_top_level_bindings, collect_type_only_import_bindings, collect_typed_top_level_lets,
    collect_typed_uninit_lets, find_store_refs, find_store_refs_with_bindings,
};
pub use template_refs::find_template_refs;
pub use template_walker::{
    BindThisCheck, BindThisTarget, ComponentInstantiation, DomBinding, DomBindingExpression,
    OnEventDirective, PropShape, SlotAttrExpr, SlotDef, TemplateSummary, literal_attr_value,
    resolve_bind_value_type, walk_template,
};
pub use void_refs::VoidRefRegistry;
