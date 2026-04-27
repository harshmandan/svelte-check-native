//! Public data types for the typecheck pipeline.
//!
//! Pulled out of `lib.rs` for navigability — these are pure value
//! types with no logic. The orchestrator (`check`) and the
//! diagnostic mapper (`map_diagnostic`) live in `lib.rs` and consume
//! these types.

use std::path::PathBuf;

use crate::discovery::DiscoveryError;
use crate::output::Severity;
use crate::runner::RunError;
use svn_emit::{LineMapEntry, TokenMapEntry};

/// Per-file mapping data the diagnostic mapper needs to translate a
/// tsgo `(line, column)` back to a source `(line, column)`.
///
/// Built from each input's [`CheckInput`] fields (line_map + token_map +
/// overlay_line_starts + source_line_starts) and keyed by the overlay
/// path so diagnostic lookup is O(1) on path.
#[derive(Debug, Clone, Default)]
pub struct MapData {
    pub line_map: Vec<LineMapEntry>,
    pub token_map: Vec<TokenMapEntry>,
    pub overlay_line_starts: Vec<u32>,
    pub source_line_starts: Vec<u32>,
    /// Overlay text. Required because tsgo emits 1-based UTF-16
    /// column counts (LSP convention) and we need the actual line
    /// contents to convert UTF-16 column → byte offset. Pure ASCII
    /// lines treat both as equivalent; non-ASCII lines diverge.
    pub overlay_text: String,
    /// Source `.svelte` text. Same UTF-16-vs-byte conversion need on
    /// the source side: we map a matched token-map's source byte
    /// range back to a (line, UTF-16-column) for the user-facing
    /// diagnostic.
    pub source_text: String,
    /// When true, overlay positions that don't match any `token_map` /
    /// `line_map` entry pass through unchanged (identity map) instead
    /// of being dropped. Set for kit-file inputs where the overlay is
    /// the original source plus sparse `: T` insertions that never add
    /// lines — diagnostics against unmodified regions line up 1:1.
    pub identity_map: bool,
    /// Byte-offset ranges (start, end) in the overlay where emit has
    /// marked scaffolding with `IGNORE_START_MARKER` / `IGNORE_END_MARKER`.
    /// Diagnostics whose start position falls inside any of these
    /// ranges are dropped in `map_diagnostic`. Ranges are sorted by
    /// start and non-overlapping (each `ignore_start` pairs with the
    /// NEXT `ignore_end`).
    pub ignore_regions: Vec<(u32, u32)>,
}

/// One file to type-check.
#[derive(Debug, Clone)]
pub struct CheckInput {
    /// Original source path (absolute). Usually a `.svelte` file; for
    /// Kit-file inputs (`kind == InputKind::KitFile`) it's a `.ts`
    /// under `src/routes/` or `src/hooks.*` / `src/params/`.
    pub source_path: PathBuf,
    /// Generated TypeScript that should be type-checked.
    pub generated_ts: String,
    /// Line mappings from emit — overlay-line ranges back to source-line
    /// ranges. Empty for non-Svelte inputs (where overlay line == source
    /// line).
    pub line_map: Vec<LineMapEntry>,
    /// Token-level byte-span mappings from emit — synthesized overlay
    /// spans back to source byte spans. Takes precedence over
    /// `line_map` during diagnostic translation. Empty for Kit files
    /// and currently empty for Svelte files too (v0.3 Item 1 is pure
    /// plumbing; emit sites start pushing in follow-up PRs).
    pub token_map: Vec<TokenMapEntry>,
    /// Byte offsets of each line's start in the generated overlay. Used
    /// to translate a tsgo `(line, column)` into an overlay byte
    /// offset for token-map lookup. Empty for Kit files (line-col
    /// pass-through is correct there).
    pub overlay_line_starts: Vec<u32>,
    /// Byte offsets of each line's start in the `.svelte` source. Used
    /// to translate a matched TokenMapEntry's `source_byte_start`
    /// back into a source `(line, column)`. Empty for Kit files.
    pub source_line_starts: Vec<u32>,
    /// What flavor of input this is. Drives the cache-write layout
    /// (`.svelte.svn.ts` overlay + ambient sidecar for Svelte files;
    /// mirror `.ts` in the cache-svelte tree for Kit files) and the
    /// overlay-tsconfig treatment (Svelte files emit an exclusion
    /// for the original `.svelte`; Kit files add the original `.ts`
    /// to `exclude` so tsgo only sees our injected-type overlay).
    pub kind: InputKind,
    /// Whether the generated overlay is TypeScript (`.svelte.svn.ts`)
    /// or JavaScript (`.svelte.svn.js`). True for Kit/UserTsOverlay
    /// kinds (always TS) and for Svelte sources whose
    /// `Document::script_lang()` resolves to `Ts`. False only when
    /// the JS-overlay branch is enabled AND the Svelte source has no
    /// `<script lang="ts">`. The tsgo-applied inference rules differ
    /// per extension under `checkJs:true + noImplicitAny:false`
    /// (a common Svelte-5 CMS-style tsconfig shape): `.js` widens
    /// `let x = $state([])` to `any[]`, `.ts` keeps it `never[]` —
    /// load-bearing for third-party-integration clusters like the
    /// CodeMirror.svelte wrapper pattern.
    pub is_ts_overlay: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputKind {
    /// Svelte file that's IN the user's tsconfig scope
    /// (`files` / `include` minus `exclude`). Written to the cache
    /// AND listed explicitly in the overlay tsconfig's `files` so
    /// tsgo loads it unconditionally.
    Svelte,
    /// Svelte file discovered on disk but NOT in the user's tsconfig
    /// scope. Written to the cache + ambient sidecar so tsgo's
    /// import resolution finds the overlay when an in-scope file
    /// does `import './Foo.svelte'`, but NOT listed in the overlay
    /// tsconfig's `files`. Mirrors upstream svelte2tsx's pattern of
    /// emitting every discovered Svelte file and letting the
    /// LanguageService follow imports to decide what gets checked.
    SvelteAuxiliary,
    /// SvelteKit Kit file (`+server.ts`, `+page.ts`, hooks, params)
    /// that went through `kit_inject`. Writes a MIRROR overlay at
    /// the same relative path under the cache svelte dir and pushes
    /// the original source path into the overlay tsconfig's
    /// `exclude` list so tsgo reads only the typed version.
    KitFile,
    /// User-authored `.ts` file that statically imports at least one
    /// `.svelte` component whose directory ALSO contains a sibling
    /// `.svelte.ts` runes module (the collision case that makes
    /// tsgo's `rootDirs` resolution pick the runes module instead of
    /// our overlay). We emit a mirror overlay at `kit_overlay_path`
    /// with every `.svelte` specifier rewritten to `.svelte.svn.js`,
    /// so the overlay resolves directly to the cache's generated TS.
    /// Original source path is pushed into `exclude` so tsgo reads
    /// only the rewritten version.
    UserTsOverlay,
}

/// A single mapped-back diagnostic ready for presentation.
#[derive(Debug, Clone)]
pub struct CheckDiagnostic {
    /// Original `.svelte` (or `.ts`/`.js`) source path.
    pub source_path: PathBuf,
    /// 1-based line of the diagnostic START in the original source.
    pub line: u32,
    /// 1-based column of the diagnostic START in the original source.
    pub column: u32,
    /// 1-based line of the diagnostic END. Equal to `line` for
    /// single-line spans.
    pub end_line: u32,
    /// 1-based column of the diagnostic END (exclusive). For zero-width
    /// spans this equals `column`.
    pub end_column: u32,
    pub severity: Severity,
    /// Code identifier. `Numeric` for TypeScript (TS6133, TS2614, …),
    /// `Slug` for Svelte compiler warnings (`state_referenced_locally`,
    /// `element_invalid_self_closing_tag`, …). The output formatter
    /// emits each form natively (number / quoted string).
    pub code: DiagnosticCode,
    pub message: String,
    /// Where this diagnostic came from. Drives the `source` field in
    /// machine-output and matches upstream svelte-check's classification.
    pub source: DiagnosticSource,
    /// Documentation URL for this diagnostic, if available. Surfaces as
    /// `codeDescription.href` in machine-verbose output — IDE
    /// integrations render it as a clickable link in the problems
    /// panel.
    pub code_description_url: Option<String>,
}

/// Polymorphic diagnostic code: TypeScript uses numbers, the Svelte
/// compiler uses string slugs. Upstream svelte-check emits each
/// natively in machine output (`"code": 6133` vs
/// `"code": "state_referenced_locally"`), so editors and CI parsers
/// can route by type.
#[derive(Debug, Clone)]
pub enum DiagnosticCode {
    Numeric(u32),
    Slug(String),
}

impl std::fmt::Display for DiagnosticCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            // TS-style display: `TS6133`. The numeric form is what
            // user-facing tooling typically expects when prefixed.
            Self::Numeric(n) => write!(f, "TS{n}"),
            // Compiler slugs render as-is — matches the way svelte-
            // check shows `state_referenced_locally`.
            Self::Slug(s) => f.write_str(s),
        }
    }
}

/// Diagnostic origin. Mirrors the `source` field upstream svelte-check
/// emits for each diagnostic (`"js"` covers both TS and JS — same
/// backend; `"svelte"` is compiler diagnostics; `"css"` is CSS-linter
/// diagnostics, reserved here for when we add a CSS pass).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DiagnosticSource {
    Js,
    Svelte,
    Css,
}

impl DiagnosticSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Js => "js",
            Self::Svelte => "svelte",
            Self::Css => "css",
        }
    }
}

/// Errors from the full check pipeline.
#[derive(Debug, thiserror::Error)]
pub enum CheckError {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("tsgo discovery: {0}")]
    Discovery(#[from] DiscoveryError),

    #[error("tsgo run: {0}")]
    Run(#[from] RunError),

    #[error("failed to serialize overlay tsconfig: {0}")]
    Serialize(#[from] serde_json::Error),
}

/// Bundle of what [`crate::check`] returns.
#[derive(Debug)]
pub struct CheckOutput {
    pub diagnostics: Vec<CheckDiagnostic>,
    /// tsgo's `--extendedDiagnostics` stats block, present iff the
    /// caller passed `extended_diagnostics = true` AND tsgo emitted a
    /// recognizable block. The CLI prints this verbatim after the
    /// normal output so users see tsgo's native perf/memory stats.
    pub extended_diagnostics: Option<String>,
}

/// Marker (start) for emit-synthesised regions whose diagnostics
/// should be muted. See `MapData::ignore_regions`.
pub const IGNORE_START_MARKER: &str = "/*svn:ignore_start*/";
/// Marker (end) for emit-synthesised regions whose diagnostics should
/// be muted.
pub const IGNORE_END_MARKER: &str = "/*svn:ignore_end*/";
