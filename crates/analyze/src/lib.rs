//! Semantic analysis passes over the Svelte AST.
//!
//! Two product-type outputs are produced and threaded independently
//! through emit (no single bundling struct):
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
//! In addition to the bundled outputs, the crate exports several
//! stateful accumulator helpers from `store`
//! (`collect_top_level_bindings`, `collect_type_only_import_bindings`,
//! `find_store_refs`, `find_store_refs_with_bindings`,
//! `collect_typed_uninit_lets`, `collect_typed_top_level_lets`) plus
//! `find_template_refs`.
//! These are driven by emit at specific points in its flow (e.g.
//! `collect_top_level_bindings` is called three times to union
//! identifiers from module + instance + rewritten-instance
//! programs). They join `SemanticModel` only when a second consumer
//! needs the same accumulated set up-front — see `CLAUDE.md`'s
//! "don't invent placeholder fields with no reader" rule. (Props and
//! the template summary are passed to emit as separate arguments
//! rather than wrapped together.)
//!
//! All passes share a single `Visitor` walk of the AST. One pass,
//! many collectors.

// Tests are allowed to panic loudly on setup failures.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod ast_walk;
pub mod dom_binding;
pub mod events;
pub mod jsdoc;
pub mod nodes;
pub mod props;
pub mod slot_attr_rewrite;
pub mod store;
pub mod template_refs;
pub mod template_scope;
pub mod void_refs;
pub mod walker;

pub use ast_walk::{WalkNode, collect_function_body_stmts, walk_statement_descend};
pub use jsdoc::{
    scan_jsdoc_props_typedef_keys, scan_jsdoc_typedef_name, should_synthesise_js_props,
};
pub use nodes::attribute::literal_attr_value;
pub use nodes::binding::resolve_bind_value_type;
pub use events::{
    collect_ctor_locals, collect_inline_typed_dispatcher_member_names, find_dispatched_event_names,
    find_dispatcher_event_type_sources, find_dispatcher_local_names,
    find_typed_dispatcher_local_names, find_untyped_dispatcher_local_names,
    has_event_dispatcher_call, has_inline_typed_dispatcher_members,
};
pub use props::{PropInfo, PropsInfo, PropsSource, contains_typeof_ref, root_type_name_of};
pub use store::{
    collect_top_level_bindings, collect_type_only_import_bindings, collect_typed_top_level_lets,
    collect_typed_uninit_lets, find_store_refs, find_store_refs_with_bindings,
};
pub use template_refs::find_template_refs;
pub use template_scope::extract_at_const_bindings;
pub use void_refs::VoidRefRegistry;
pub use walker::{
    BindDirective, BindThisTarget, BubbledComponentEvent, BubbledDomEvent, BubbledDomEventScope,
    ComponentInstantiation, OnEventDirective, PropShape, ResolvedSlotExpr, SlotAttr, SlotAttrExpr,
    SlotDef, TemplateSummary, walk_template,
};
