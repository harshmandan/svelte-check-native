//! Filename-parity stubs for upstream
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/utils/`.
//!
//! Three upstream files: `tsAst.ts`, `Scope.ts`, `error.ts`. Each is a
//! pointer to where the equivalent concern lives in our tree. None of
//! upstream's helpers map 1:1 — TypeScript-AST traversal, scope
//! analysis, and error formatting each take a different shape in Rust.
//!
//! This directory exists as a navigation aid for contributors familiar
//! with upstream's layout: `svelte2tsx/utils/<X>.ts` lands here at
//! `svelte2tsx_utils/<x>.rs`.

pub(crate) mod error;
pub(crate) mod scope;
pub(crate) mod ts_ast;
