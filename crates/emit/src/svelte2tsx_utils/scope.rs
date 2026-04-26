//! Lexical-scope tracking used by script-wrapping passes.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/utils/Scope.ts`.
//!
//! **Status: handled at a different layer.**
//!
//! Upstream's `Scope` class tracks instance-script bindings and store
//! subscriptions (`$NAME` auto-subscribe), and is shared by
//! `processInstanceScriptContent.ts`, `ImplicitStoreValues.ts`, and
//! `ExportedNames.ts`. The shared analysis is done up front and read
//! by each emit-time consumer.
//!
//! Our equivalent is split across two layers:
//!
//! 1. **Template scope** — `crates/analyze/src/template_scope.rs` —
//!    tracks `{#each}` context bindings, `{#snippet}` parameters,
//!    `let:` directive names, `{@const}` declarations. This is the
//!    Svelte-template-side of "what names are in scope here".
//! 2. **Script-side scope** — handled at emit time via
//!    [`crate::process_instance_script_content`]'s walk of the
//!    instance script. We don't carry an explicit `Scope` struct; the
//!    walk's locals are tracked inline via `oxc_ast` visits.
//!
//! Cross-cutting concerns that upstream's Scope feeds:
//!
//! | Upstream concern | Our equivalent |
//! |---|---|
//! | Store subscription discovery (`$NAME`) | [`svn_analyze::find_store_refs_with_bindings`] |
//! | Exported name tracking | [`crate::process_instance_script_content::SplitScript::exported_locals`] |
//! | Generic parameter extraction | [`crate::util::extract_generics_attr`] + [`crate::util::generic_arg_names`] |
//! | Reactive-statement target collection | inline in [`crate::svelte4::compat`] |
//!
//! The split (template vs script scope) is intentional. Template scope
//! is per-walk state for the dispatcher; script scope is mostly a
//! one-shot analysis whose results are consumed at emit-time. Joining
//! them into a single `Scope` struct would conflate two lifetimes.
//!
//! This file is a navigational stub only.
