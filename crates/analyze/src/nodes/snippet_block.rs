//! `{#snippet}` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/SnippetBlock.ts`.
//!
//! Nothing to record: svelte2tsx's slot resolver (`slot.ts`) does not
//! track snippet parameters, so a `<slot>` attribute naming one keeps
//! the name as written. The walker's `enter_scope` therefore pushes no
//! resolver entries for the snippet scope.
