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
//! ### Design notes
//!
//! - `Range` is the byte-offset interval type — name aligns with LSP.
//! - `PositionMap` resolves byte offsets to line/col on demand; does not
//!   conflate source-map-v3 mappings with line/col tables. Source maps
//!   live in the `emit` crate where they're actually constructed.
//! - One canonical `Diagnostic` type — no parallel representations in
//!   different stages of the pipeline.

pub mod diagnostic;
pub mod position;
pub mod range;
pub mod symbol;
pub mod tsconfig;

/// The literal `node_modules` directory name. Crates that walk up the
/// directory tree looking for installed packages or that filter out
/// `node_modules` paths reference this rather than re-spelling the
/// string. Centralising here keeps `is_excluded_dir` filters,
/// package-discovery routines, and overlay path handling in lockstep.
pub const NODE_MODULES_DIR: &str = "node_modules";

/// Walk up the directory tree starting at `start`, calling `probe` on
/// each ancestor. Returns the first `Some(_)` the probe yields, or
/// `None` if the chain reaches the filesystem root without a hit.
///
/// Used for the "find a package, config, or marker file in this
/// project's resolution chain" pattern that recurs across crates
/// (`locate_svelte`, `has_real_svelte`, tsgo discovery, tsconfig
/// search, runtime types resolution).
///
/// `probe` is called on `start` first, then on each ancestor in order
/// — same behaviour as the hand-rolled `cur = dir.parent()` loops it
/// replaces.
pub fn walk_up_dirs<F, T>(start: &std::path::Path, mut probe: F) -> Option<T>
where
    F: FnMut(&std::path::Path) -> Option<T>,
{
    let mut cur: Option<&std::path::Path> = Some(start);
    while let Some(dir) = cur {
        if let Some(found) = probe(dir) {
            return Some(found);
        }
        cur = dir.parent();
    }
    None
}

// Re-exports so consumers can write `svn_core::Range` etc.
pub use diagnostic::{Diagnostic, DiagnosticSource, Severity};
pub use position::{Position, PositionMap};
pub use range::Range;
pub use symbol::Symbol;
pub use tsconfig::{CompilerOptions, ModuleResolution, Reference, TsConfigFile};
