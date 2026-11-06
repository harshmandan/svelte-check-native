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
mod compile_options;
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
mod transpile_sensitive;
mod walk;

use std::path::Path;

pub use codes::{CODES, Code};
pub use compat::{CompatFeatures, SvelteVersion, detect_for_workspace};
pub use compile_options::{CompileOptionsCheck, LateOption, OptionValue, check_compile_options};
pub use context::{LintContext, Warning};
pub use parse_rejection::{script_tag_rejected, template_parse_rejected};
pub use rules::bidi_state::{BidiTrace, RegexUse};

/// The project compiler options the pass honours (from the nearest
/// Svelte config).
#[derive(Debug, Clone, Default)]
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
    /// What the compiler's validation of the config's
    /// `compilerOptions` reports (see [`check_compile_options`]).
    pub compile_options: Option<std::sync::Arc<CompileOptionsCheck>>,
    /// The compile options' warnings this component shows. The compiler
    /// warns about an option once per process, so only the first
    /// component compiled with it shows each (the caller decides which).
    pub compile_option_warnings: Vec<(Code, String)>,
    /// The `lastIndex` the compiler's bidirectional-character regex
    /// holds when this component is compiled: it is shared by every
    /// compile in the process (see [`LintReport::bidi`]).
    pub bidi_last_index: u32,
}

/// Report the compile options' validation before anything else: its
/// warnings (on the first component only) and its error, which the
/// compiler throws before reading the component.
fn report_compile_options(options: &LintOptions, ctx: &mut LintContext<'_>) {
    let at = svn_core::Range::new(0, 0);
    for (code, message) in &options.compile_option_warnings {
        ctx.emit(*code, message.clone(), at);
    }
    let Some(check) = &options.compile_options else {
        return;
    };
    if let Some((code, message)) = &check.error {
        ctx.emit_error(*code, message.clone(), at);
    }
    ctx.compile_options_late_error = check.late_error.clone();
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
    report_compile_options(&options, &mut ctx);
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
) -> LintReport {
    let mut ctx = LintContext::with_positions(source, positions);
    ctx.compat = compat;
    ctx.experimental_async = options.experimental_async;
    ctx.ts_scripts_transpiled = options.ts_scripts_transpiled;
    ctx.preprocess_configured = options.preprocess_configured;
    ctx.bidi_last_index = options.bidi_last_index;
    report_compile_options(&options, &mut ctx);
    crate::walk::walk_parsed(doc, fragment, source, path, options.runes, &mut ctx);
    let exception = ctx.take_exception();
    let needs_real_transpile = ctx.needs_real_transpile && !ctx.real_transpile_unavailable;
    // A compile that throws never reaches the analysis, where the regex
    // is used.
    let bidi = if ctx.errored {
        BidiTrace::Untouched
    } else {
        std::mem::take(&mut ctx.bidi_trace)
    };
    LintReport {
        warnings: ctx.take_warnings(),
        exception,
        needs_real_transpile,
        bidi,
    }
}

/// What the compile-warning pass found in one component.
#[derive(Debug, Clone, Default)]
pub struct LintReport {
    /// The compiler's warnings, or the one compile error it threw.
    pub warnings: Vec<Warning>,
    /// The message of the JavaScript exception the compiler crashed
    /// with instead of compiling the component. svelte-check reports
    /// it as an error with no code, at the start of the file.
    pub exception: Option<String>,
    /// The warnings come from modelling TypeScript's transpile, and the
    /// component has something that model cannot follow: the caller
    /// should lint the real transpiled component instead (see
    /// `transpile_sensitive`). Only set when the project has no Svelte
    /// config, so the language server's fallback TypeScript transpile
    /// is the preprocessor.
    pub needs_real_transpile: bool,
    /// What compiling this component does to the compiler's shared
    /// bidirectional-character regex, for running components in the
    /// order the compiler sees them.
    pub bidi: BidiTrace,
}
