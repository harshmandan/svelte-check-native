//! `<script generics="T extends Item, K extends keyof T">` parsing
//! and emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/Generics.ts`.
//!
//! **Status: split between parser, util, and default_export.**
//!
//! Upstream's `Generics` class:
//!
//! 1. Parses the `generics="..."` attribute value off the instance
//!    `<script>` tag.
//! 2. Splices it verbatim into the render-function declaration:
//!    `function $$render<T extends Item, K extends keyof T>() { … }`.
//! 3. Reduces `T extends Item, K extends keyof T` to just the names
//!    (`T, K`) at instantiation sites:
//!    `typeof $$render<T, K>`.
//! 4. Generates the `<any, any, …>` substitution used inside the
//!    `$$IsomorphicComponent` interface's `z_$$bindings?` field
//!    (interface members can't reference the interface's own free
//!    type parameter).
//!
//! Our equivalent:
//!
//! - **Parsing** the `generics=""` attribute happens in
//!   `svn_parser::ScriptSection::generics` (the structural parser
//!   captures the attribute value at parse time).
//! - **Verbatim splicing** at the declaration site is done by
//!   [`crate::util::extract_generics_attr`] in `lib.rs`'s
//!   `emit_document_with_render_name`.
//! - **Name reduction** for instantiation-site references is
//!   [`crate::util::generic_arg_names`].
//! - **Class-wrapper declaration** (`declare class
//!   __svn_Render_<hash><T> { … }`) and the `<any, any, …>`
//!   substitution for `$$bindings` are built by
//!   [`crate::default_export::emit_default_export_declarations_ts`].
//!
//! This file exists for parity navigation.
