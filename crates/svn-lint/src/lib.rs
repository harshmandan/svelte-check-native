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
//! - [`lint_file`] — run the full warning pass on one file (parses
//!   internally).
//! - [`lint_parsed`] — run the pass on an already-parsed document,
//!   reusing the caller's parse + position map.

mod a11y_constants;
mod aria_data;
mod codes;
mod compat;
mod context;
mod fuzzymatch;
// The vendored HTML5 tree-validation tables live in `svn-parser` (the
// parser needs `closing_tag_omitted` for implicit-close handling); this
// alias keeps the `crate::html5::*` paths working here unchanged.
use svn_parser::html5;
mod ignore;
mod messages;
mod parse_errors;
mod parse_rejection;
mod rules;
mod scope;
mod scope_rune_detection;
// `scope_types` holds the public data types (Binding, Scope, …);
// `scope` re-exports them, so callers reach them as
// `crate::scope::Binding` etc. unchanged.
mod scope_types;
mod scope_util;
mod walk;

use std::path::Path;

pub use codes::{CODES, Code};
pub use compat::{CompatFeatures, SvelteVersion, detect_for_workspace};
pub use context::{LintContext, Warning};
pub use parse_rejection::{script_tag_rejected, template_parse_rejected};

/// The project compiler options the pass honours (from the nearest
/// Svelte config).
#[derive(Debug, Clone, Copy, Default)]
pub struct LintOptions {
    /// `compilerOptions.runes`: forces the mode when set.
    pub runes: Option<bool>,
    /// `compilerOptions.experimental.async`.
    pub experimental_async: bool,
    /// The project's preprocessors transpile `<script lang="ts">`
    /// bodies to JavaScript before the compiler runs.
    pub ts_scripts_transpiled: bool,
    /// The project's Svelte config supplies its own `preprocess` (the
    /// language server's fallback preprocessor does not count).
    pub preprocess_configured: bool,
}

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
    let options = LintOptions {
        runes,
        ..LintOptions::default()
    };
    lint_file_with_options(source, path, options, compat)
}

/// [`lint_file`] with the full set of project compiler options.
pub fn lint_file_with_options(
    source: &str,
    path: &Path,
    options: LintOptions,
    compat: CompatFeatures,
) -> Vec<Warning> {
    let mut ctx = LintContext::new(source);
    ctx.compat = compat;
    ctx.experimental_async = options.experimental_async;
    ctx.ts_scripts_transpiled = options.ts_scripts_transpiled;
    ctx.preprocess_configured = options.preprocess_configured;
    // `walk` resolves runes mode from the document it parses (reusing
    // that parse) — pass the caller's hint through rather than running
    // a separate `infer_runes_mode` parse here.
    crate::walk::walk(source, path, options.runes, &mut ctx);
    ctx.take_warnings()
}

/// Run the warning pass on an ALREADY-PARSED document.
///
/// The CLI's fused native pass parses each `.svelte` file once — for
/// both fatal-compile-error detection and this lint walk — and builds
/// one [`PositionMap`](svn_core::PositionMap) per file. This entry lets
/// it hand the parse (`doc` + `fragment`) and the map straight in,
/// instead of [`lint_file`] re-parsing and re-indexing the source.
/// `options.runes`/`compat` behave as `runes`/`compat` in [`lint_file`].
pub fn lint_parsed<'src>(
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::ast::Fragment,
    source: &'src str,
    positions: svn_core::PositionMap<'src>,
    path: &Path,
    options: LintOptions,
    compat: CompatFeatures,
) -> Vec<Warning> {
    let mut ctx = LintContext::with_positions(source, positions);
    ctx.compat = compat;
    ctx.experimental_async = options.experimental_async;
    ctx.ts_scripts_transpiled = options.ts_scripts_transpiled;
    ctx.preprocess_configured = options.preprocess_configured;
    crate::walk::walk_parsed(doc, fragment, source, path, options.runes, &mut ctx);
    ctx.take_warnings()
}
