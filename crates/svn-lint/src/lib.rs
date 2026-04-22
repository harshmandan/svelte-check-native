#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

//! Native Svelte compile-warning lint pass.
//!
//! Reproduces `svelte/compiler`'s `compile(..., { generate: false })`
//! warning set in Rust so we can drop the multi-worker Node bridge.
//!
//! Architecture:
//!
//! - **Code catalog** (`codes.rs`, `messages.rs`) — generated from
//!   `.svelte-upstream/svelte/packages/svelte/messages/compile-warnings/*.md`
//!   by `cargo run -p xtask --bin regen-lint-catalog`. Every known warning
//!   has a stable `Code` enum variant + a message-building function.
//!
//! - **`LintContext`** — per-file state: warnings sink, ignore stack,
//!   ignore map (for post-walk fires), source text, position map. Mirrors
//!   upstream's module-global `state.js` but per-call.
//!
//! - **Rules** (`rules/*.rs`) — one module per warning family. Each
//!   exports functions that take a node + the context and push warnings
//!   when the pattern fires. Rules are called from a single walker that
//!   traverses the template AST + JS/TS AST in one pass.
//!
//! - **Ignore stack** — `<!-- svelte-ignore CODE -->` comment handling
//!   mirrors `utils/extract_svelte_ignore.js` byte-for-byte (legacy
//!   code renames, runes-mode comma separators, fuzzymatch
//!   suggestions).
//!
//! Public entry points:
//!
//! - [`lint_file`] — run the full warning pass on one file.
//! - [`lint_batch`] — parallel over many files (rayon).

pub mod a11y_constants;
pub mod aria_data;
pub mod codes;
pub mod compat;
pub mod context;
pub mod html5;
pub mod ignore;
pub mod messages;
pub mod rules;
pub mod scope;
pub mod walk;

use std::path::{Path, PathBuf};

pub use codes::{CODES, Code};
pub use compat::{CompatFeatures, SvelteVersion};
pub use context::{LintContext, Warning};

/// Run the compile-warning pass on one source file.
///
/// `source` is the raw `.svelte` file contents; `path` is informational
/// and used only for diagnostic output. `runes` selects runes mode;
/// if `None` it's auto-detected following upstream's logic (instance
/// script contains a rune reference or filename is `.svelte.{js,ts}`).
/// `compat` gates rules that evolved across svelte versions; pass
/// [`CompatFeatures::MODERN`] when the user's svelte version is
/// unknown (matches what the upstream validator suite enforces).
pub fn lint_file(
    source: &str,
    path: &Path,
    runes: Option<bool>,
    compat: CompatFeatures,
) -> Vec<Warning> {
    let mut ctx = LintContext::new(source);
    ctx.runes = runes.unwrap_or_else(|| crate::walk::infer_runes_mode(source, path));
    ctx.compat = compat;
    crate::walk::walk(source, &mut ctx);
    ctx.take_warnings()
}

/// Batch entry: lint many files in parallel.
///
/// Returns `(path, warnings)` pairs in arbitrary order. Callers sort/
/// flatten as needed for display. `compat` is applied to every file
/// in the batch.
pub fn lint_batch<I>(inputs: I, compat: CompatFeatures) -> Vec<(PathBuf, Vec<Warning>)>
where
    I: IntoIterator<Item = (PathBuf, String)>,
    I::IntoIter: Send,
{
    use rayon::iter::{IntoParallelIterator, ParallelIterator};
    let items: Vec<(PathBuf, String)> = inputs.into_iter().collect();
    items
        .into_par_iter()
        .map(|(path, source)| {
            let warnings = lint_file(&source, &path, None, compat);
            (path, warnings)
        })
        .collect()
}
