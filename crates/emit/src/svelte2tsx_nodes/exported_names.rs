//! Exported-name tracking from instance / module scripts.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts`.
//!
//! **Status: handled by `process_instance_script_content::ExportedLocalInfo`.**
//!
//! Upstream's `ExportedNames` is a stateful class with methods like
//! `addExportedName`, `getExportsMap`, `createReturnElementsType`. It
//! serves both:
//!
//! 1. **Emit-time** — building the `return { exports: { … } }` field
//!    of the render function and the corresponding intersection on
//!    the default-export type.
//! 2. **LSP-time** — exposing `getExportsMap()` so the language-server
//!    can answer "go to definition" for each exported name.
//!
//! Our equivalent serves only #1 (we're CLI-only):
//!
//! - The `process_instance_script_content` pass collects every
//!   `export let` / `export function` / `export const` /
//!   `export { a as b }` / `export type` / `export interface` into
//!   `SplitScript::exported_locals` and `export_type_infos`.
//!   See [`crate::process_instance_script_content`].
//! - The render function uses `build_exports_object` to assemble the
//!   `{ name: T; … }` object-type literal that backs
//!   `Awaited<ReturnType<typeof $$render>>['exports']`. See
//!   [`crate::props_emit::build_exports_object`].
//! - The default-export shape consumes that projection at module
//!   scope. See [`crate::default_export::emit_default_export_declarations_ts`].
//!
//! No `getExportsMap()` analog exists because we have no LSP consumer.
//! If LSP support lands, the natural extension is to add an
//! `ExportedLocalInfo::to_lsp_map()` method on the existing struct.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `ExportedNames.ts` should land here and find the
//! pointers above.
