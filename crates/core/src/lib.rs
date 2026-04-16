// Tests are allowed to `.expect()` / `.unwrap()`: they're supposed to panic
// loudly on unexpected states. The library code keeps both lints as warnings.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! Shared primitives for svelte-check-native.
//!
//! This crate is the foundation every other crate in the workspace depends
//! on. It is kept small, allocation-light, and free of I/O: pure types +
//! deterministic helpers only.
//!
//! ### Module map
//!
//! - [`range`] — `Range` (byte-offset half-open interval, `u32` bounds).
//! - [`position`] — `Position`, `PositionMap`. LSP-compatible line/col
//!   resolution with UTF-16 code-unit column counting.
//! - [`symbol`] — `Symbol`, a string interning type (currently re-exports
//!   `smol_str::SmolStr`).
//! - [`diagnostic`] — `Diagnostic`, `Severity`, `DiagnosticSource`.
//!
//! ### Design notes vs. `-rs`
//!
//! - `Range` replaces `Span` (name aligns with LSP).
//! - `PositionMap` replaces `SourceMap`; it no longer conflates source-map-v3
//!   mappings with line/col tables. The source-map-v3 work lives in the
//!   `emit` crate where it's actually used.
//! - One canonical `Diagnostic` type — no parallel representations in
//!   different stages of the pipeline.

pub mod diagnostic;
pub mod position;
pub mod range;
pub mod symbol;

// Re-exports so consumers can write `svn_core::Range` etc.
pub use diagnostic::{Diagnostic, DiagnosticSource, Severity};
pub use position::{Position, PositionMap};
pub use range::Range;
pub use symbol::Symbol;
