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

pub mod props;
pub mod rune;
pub mod store;
pub mod template_refs;
pub mod template_walker;
pub mod void_refs;

pub use props::{PropInfo, find_props};
pub use rune::{RuneCall, RuneKind, find_runes};
pub use store::{collect_top_level_bindings, find_store_refs, find_store_refs_with_bindings};
pub use template_refs::find_template_refs;
pub use template_walker::{BindThisTarget, TemplateSummary, walk_template};
pub use void_refs::VoidRefRegistry;
