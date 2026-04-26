//! Module-script (`<script context="module">` / `<script module>`)
//! processing.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/processModuleScriptTag.ts`.
//!
//! **Status: handled inline by `process_instance_script_content.rs`.**
//!
//! Unlike upstream — which has two dedicated modules
//! (`processInstanceScriptContent.ts` for the regular `<script>` block
//! and `processModuleScriptTag.ts` for `<script context="module">`) —
//! we currently process both kinds of scripts in
//! [`crate::process_instance_script_content`]. The split is light and
//! the helpers are shared (import hoisting, export tracking, type
//! extraction), so combining them is more idiomatic for the structural
//! emit.
//!
//! What `processModuleScriptTag.ts` does that we mirror inline:
//!
//! - **Hoist `<script module>` to module top level.** Module-script
//!   declarations are already module-scope; we just include them
//!   verbatim in the overlay before the `$$render_<hash>` wrapping
//!   function. See `lib.rs::emit_document_with_render_name` for the
//!   ordering.
//! - **Strip `context="module"` / `module` attributes from emit.** No
//!   action required since we don't re-emit the `<script>` opening tag.
//! - **Track names declared in module scope** for the
//!   `module_script_declares_type` predicate (used by the default-export
//!   shape to decide whether a Props named-type reference is module-
//!   scope-visible). This lives in
//!   [`crate::default_export::emit_default_export_declarations_ts`].
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `processModuleScriptTag.ts` should land here and find
//! the pointers above.
