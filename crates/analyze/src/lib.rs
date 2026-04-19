//! Semantic analysis passes over the Svelte AST.
//!
//! Populates a `SemanticModel` with: detected runes, prop destructures,
//! store subscriptions, `bind:` targets, SvelteKit route role, and —
//! critically — a `VoidRefRegistry` collecting every synthesized name
//! that the emit crate will need to reference.
//!
//! Centralizing the registry of synthesized names is what stops emit from
//! having to remember a per-feature `void <name>;` line every time a new
//! emission landed. Every kind of synthesized name (template-check
//! wrapper, action attrs, bind pairs, store aliases, prop locals)
//! registers here; emit reads the registry once and writes a single
//! consolidated `void (...)` block.
//!
//! All passes share a single `Visitor` walk of the AST. One pass, many
//! collectors.

// Tests are allowed to panic loudly on setup failures.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod dom_binding;
pub mod props;
pub mod rune;
pub mod store;
// SVELTE-4-COMPAT: droppable submodule for Svelte-4 syntax handling.
// See design/phase_g/DESIGN.md.
pub mod svelte4;
pub mod template_refs;
pub mod template_walker;
pub mod void_refs;

pub use props::{PropInfo, find_props, find_props_type_source};
pub use rune::{RuneCall, RuneKind, find_runes};
pub use store::{
    collect_top_level_bindings, collect_typed_top_level_lets, collect_typed_uninit_lets,
    find_store_refs, find_store_refs_with_bindings,
};
pub use template_refs::find_template_refs;
pub use template_walker::{
    BindThisCheck, BindThisTarget, ComponentInstantiation, DomBinding, DomBindingExpression,
    OnEventDirective, PropShape, TemplateSummary, literal_attr_value, resolve_bind_value_type,
    walk_template,
};
pub use void_refs::VoidRefRegistry;
