//! Shared primitives for svelte-check-native.
//!
//! Contains byte-offset ranges, a lazy line/column position map, the canonical
//! `TsConfig` struct, the `Diagnostic` type, and the diagnostic-source enum.
//! Every other crate in the workspace depends on this one.
//!
//! Design notes:
//! - `Range { start: u32, end: u32 }` replaces `-rs`'s `Span`. `u32` byte
//!   offsets are sufficient for any real-world file (4 GiB).
//! - `PositionMap` (was `SourceMap` in `-rs`) computes line/col lazily and
//!   caches the newline index. Construct once per file, reuse.
//! - `TsConfig` is the single canonical representation. No field-by-field
//!   JSON peeking elsewhere in the workspace.
