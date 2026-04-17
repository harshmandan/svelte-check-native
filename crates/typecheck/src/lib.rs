//! tsgo integration + tsconfig overlay.
//!
//! ### Pipeline
//!
//! 1. The CLI hands us a list of `.svelte` source files plus the matching
//!    emitted TypeScript (one string per source).
//! 2. We write each generated `.svelte.ts` into the cache directory using
//!    [`cache::write_if_changed`] (so unchanged files don't perturb tsgo's
//!    incremental build info).
//! 3. We write a `.d.svelte.ts` re-export stub for each component so
//!    `import Foo from './Foo.svelte'` resolves at the type level.
//! 4. We generate an overlay tsconfig that extends the user's tsconfig
//!    and lists every generated file via `files`, plus the tsgo-mandatory
//!    `allowArbitraryExtensions: true` and `noEmit: true`.
//! 5. We spawn tsgo with `--project <overlay>.json --pretty true
//!    --noErrorTruncation`, capture combined stdout+stderr, and parse with
//!    [`output::parse`].
//! 6. We map diagnostics back to original `.svelte` paths via the cache
//!    layout. Line/column mapping is best-effort for now (we account for
//!    the wrapper offset added by [`emit::emit_document`] but don't yet
//!    have a real source map).
//!
//! ### What's not yet here
//!
//! - Proper source-map mapping (currently we apply a fixed offset matching
//!   the emitter's wrapper; once the emitter writes sourcemap mappings,
//!   this becomes a v3 source-map consumer).
//! - Path-aliased tsconfig support (`paths`/`rootDirs`/`extends-array`
//!   beyond what the user's tsconfig handles via the extends chain itself).
//! - `references` (project references) propagation.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod cache;
pub mod discovery;
pub mod output;
pub mod overlay;
pub mod runner;

use std::path::{Path, PathBuf};

pub use cache::{CacheLayout, write_if_changed};
pub use discovery::{DiscoveryError, TsgoBinary, discover};
pub use output::{RawDiagnostic, Severity, parse as parse_output};
pub use runner::{RunError, run as run_tsgo};

/// Always-on shim: Svelte 5 rune ambients ($state, $derived, $effect,
/// etc.) plus the helper types emit references (`__SvnStoreValue`,
/// `__svn_type_ref`). Written into the cache on every check.
const SVELTE_SHIMS_CORE: &str = include_str!("svelte_shims_core.d.ts");

/// Fallback `declare module 'svelte/*'` blocks for environments where
/// the user has no real `svelte` package installed (e.g. the upstream
/// fixture corpus). Written into the cache ONLY when no real svelte is
/// reachable from the workspace's node_modules chain — when one IS
/// installed, these declarations would shadow the richer real types
/// (see `svelte_shims_fallback.d.ts` header for details).
const SVELTE_SHIMS_FALLBACK: &str = include_str!("svelte_shims_fallback.d.ts");

/// Walk up from `workspace` looking for `node_modules/svelte/package.json`.
/// Returns `true` iff the user has the real `svelte` package installed
/// somewhere in the resolution chain.
fn has_real_svelte(workspace: &Path) -> bool {
    let mut cur: Option<&Path> = Some(workspace);
    while let Some(dir) = cur {
        if dir
            .join("node_modules")
            .join("svelte")
            .join("package.json")
            .is_file()
        {
            return true;
        }
        cur = dir.parent();
    }
    false
}

pub use svn_emit::LineMapEntry;

/// One file to type-check.
#[derive(Debug, Clone)]
pub struct CheckInput {
    /// Original `.svelte` source path (absolute).
    pub source_path: PathBuf,
    /// Generated TypeScript that should be type-checked.
    pub generated_ts: String,
    /// Line mappings from emit — overlay-line ranges back to source-line
    /// ranges. Empty for non-Svelte inputs (where overlay line == source
    /// line).
    pub line_map: Vec<LineMapEntry>,
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

/// Run the full type-check pipeline.
///
/// `workspace` is the user's project root; `user_tsconfig` is the absolute
/// path to their `tsconfig.json` (or `jsconfig.json`); `inputs` is the list
/// of files-to-check with their generated TS.
///
/// Consumes `inputs` so each `generated_ts` string drops as soon as it has
/// been written to the cache — the bridge phase that runs after this call
/// can hold ~100 MB of bun heaps; we don't also want to keep a duplicate
/// of every overlay TS in our own RSS waiting to be GC'd at end-of-fn.
///
/// Returns mapped diagnostics + the count of files in tsgo's program
/// (used for the COMPLETED line's `<N> FILES` denominator so it
/// matches upstream svelte-check's number — upstream prints the
/// LanguageService program count, not just the `.svelte` walker
/// count). On success with no problems the diagnostics vec is empty.
pub fn check(
    workspace: &Path,
    user_tsconfig: &Path,
    inputs: Vec<CheckInput>,
) -> Result<CheckOutput, CheckError> {
    let layout = CacheLayout::for_workspace(workspace);
    std::fs::create_dir_all(&layout.svelte_dir)?;

    // Ship the svelte type shims into the cache. Core (runes + helper
    // types) is always written. The `declare module 'svelte/*'`
    // fallback is only written when no real svelte is reachable —
    // otherwise its minimal declarations would shadow the richer real
    // types (e.g. svelte/elements re-exports HTMLAnchorAttributes,
    // HTMLInputAttributes, ClassValue from clsx, etc., none of which
    // our shim enumerates).
    let shim_text = if has_real_svelte(workspace) {
        SVELTE_SHIMS_CORE.to_string()
    } else {
        let mut combined = String::with_capacity(SVELTE_SHIMS_CORE.len() + SVELTE_SHIMS_FALLBACK.len() + 1);
        combined.push_str(SVELTE_SHIMS_CORE);
        combined.push('\n');
        combined.push_str(SVELTE_SHIMS_FALLBACK);
        combined
    };
    write_if_changed(&layout.svelte_shims, &shim_text)?;

    // Step 1: write generated TS for each input. Skip identical writes.
    // Collect a per-overlay-path line map so the diagnostic mapper can
    // translate tsgo's overlay positions back to source positions.
    //
    // Note: no separate `.d.svelte.ts` re-export stub is written. The
    // generated `<name>.svelte.ts` IS the type definition for
    // `<name>.svelte` — emit rewrites every `import './X.svelte'` to
    // `import './X.svelte.js'` so TS's standard `.js`→`.ts` resolver
    // lands on our generated file rather than the `*.svelte` ambient
    // declaration that the `svelte` package ships.
    let mut generated_paths: Vec<PathBuf> = Vec::with_capacity(inputs.len());
    let mut line_maps: std::collections::HashMap<PathBuf, Vec<LineMapEntry>> =
        std::collections::HashMap::with_capacity(inputs.len());
    // `inputs` is consumed here — `generated_ts` and `line_map` move out
    // of each `CheckInput` and the string drops at end of iteration.
    for input in inputs {
        let gen_path = layout.generated_path(&input.source_path);
        write_if_changed(&gen_path, &input.generated_ts)?;

        // Ambient `.d.svelte.ts` sidecar next to the overlay. Makes
        // `import './Foo.svelte'` in user-controlled files (barrel
        // re-exports, plain `.ts` modules we don't rewrite) resolve
        // to the overlay's types via TS's `allowArbitraryExtensions`
        // mechanism. Content is a one-shot re-export from the
        // overlay; no diagnostic should ever fire on this file.
        let ambient_path = layout.ambient_path(&input.source_path);
        let overlay_file_name = gen_path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.svelte.svn.ts");
        let ambient_text = format!(
            "// generated by svelte-check-native; do not edit\n\
             export {{ default }} from './{overlay_file_name}';\n\
             export * from './{overlay_file_name}';\n"
        );
        write_if_changed(&ambient_path, &ambient_text)?;

        line_maps.insert(gen_path.clone(), input.line_map);
        generated_paths.push(gen_path);
    }

    // Step 2: write overlay tsconfig.
    let overlay = overlay::build(&layout, user_tsconfig, &generated_paths);
    let overlay_text = serde_json::to_string_pretty(&overlay)?;
    write_if_changed(&layout.overlay_tsconfig, &overlay_text)?;

    // Step 3: spawn tsgo.
    let tsgo = discover(workspace)?;
    let run = run_tsgo(&tsgo, &layout.overlay_tsconfig, workspace)?;

    // Step 4: map diagnostics back to source paths + apply line map.
    // Drop diagnostics that are about our overlay tsconfig itself —
    // those are noise from compiler options we set deliberately
    // (e.g. TS5102 baseUrl deprecation; we filter rather than re-add
    // baseUrl because doing so suppresses tsgo's diagnostic emission
    // on our overlay files entirely).
    let diagnostics = run
        .diagnostics
        .into_iter()
        .filter(|d| !is_overlay_tsconfig_noise(d, &layout))
        .map(|d| map_diagnostic(d, &layout, &line_maps))
        .collect();
    Ok(CheckOutput {
        diagnostics,
        program_file_count: run.program_file_count,
    })
}

/// Bundle of what [`check`] returns.
#[derive(Debug)]
pub struct CheckOutput {
    pub diagnostics: Vec<CheckDiagnostic>,
    /// File count from tsgo's program (`--listFiles`). Reported in the
    /// COMPLETED line's `<N> FILES` field so the denominator matches
    /// upstream svelte-check's.
    pub program_file_count: usize,
}

/// Filter for diagnostics that come from our own overlay tsconfig and
/// represent intentional choices we've made — they're not user-actionable.
///
/// Robust against the path-shape tsgo emits: it formats diagnostic
/// paths relative to its own cwd. We set tsgo's cwd to the workspace
/// in [`run_tsgo`], so a relative `raw.file` joins back to the right
/// absolute path. As defense in depth we also accept a match by
/// canonicalized absolute path (handles symlinks like `/var` vs
/// `/private/var` on macOS) and a final ends-with check on the unique
/// `.svelte-check/tsconfig.json` suffix.
fn is_overlay_tsconfig_noise(raw: &RawDiagnostic, layout: &CacheLayout) -> bool {
    let abs = if raw.file.is_absolute() {
        raw.file.clone()
    } else {
        layout.workspace.join(&raw.file)
    };
    if abs == layout.overlay_tsconfig {
        return true;
    }
    if let (Ok(a), Ok(b)) = (abs.canonicalize(), layout.overlay_tsconfig.canonicalize()) {
        if a == b {
            return true;
        }
    }
    // Last resort: tsgo on some configurations emits the path as
    // workspace-relative even when the overlay was passed absolute.
    // The overlay's basename + parent directory together are unique
    // enough that any path matching both is ours.
    let overlay_name = layout
        .overlay_tsconfig
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let overlay_parent_name = layout
        .overlay_tsconfig
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !overlay_name.is_empty() && !overlay_parent_name.is_empty() {
        let raw_name = raw.file.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let raw_parent_name = raw
            .file
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if raw_name == overlay_name && raw_parent_name == overlay_parent_name {
            return true;
        }
    }
    false
}

fn map_diagnostic(
    raw: RawDiagnostic,
    layout: &CacheLayout,
    line_maps: &std::collections::HashMap<PathBuf, Vec<LineMapEntry>>,
) -> CheckDiagnostic {
    // tsgo emits paths relative to the working directory when the input
    // tsconfig path is itself relative (which it usually is). Absolutize
    // against the workspace root so cache-layout lookups work uniformly.
    let absolute_file = if raw.file.is_absolute() {
        raw.file.clone()
    } else {
        layout.workspace.join(&raw.file)
    };
    let (source_path, line) = match layout.original_from_generated(&absolute_file) {
        Some(orig) => {
            // Use the per-file line map if we have one; fall back to the
            // raw line clamped to >=1 if this overlay file wasn't part
            // of the current run (shouldn't happen, but be safe).
            let mapped = line_maps
                .get(&absolute_file)
                .and_then(|map| translate_line(map, raw.line))
                .unwrap_or(raw.line.max(1));
            (orig, mapped)
        }
        None => (absolute_file, raw.line),
    };
    let span = raw.span_length.unwrap_or(0);
    CheckDiagnostic {
        source_path,
        line,
        column: raw.column,
        // tsgo emits a single-line span_length, no end-line info — so
        // for TS diagnostics we collapse end_line == start_line.
        end_line: line,
        end_column: raw.column.saturating_add(span),
        severity: raw.severity,
        code: DiagnosticCode::Numeric(raw.code),
        message: raw.message,
        // Both TS and JS diagnostics from tsgo are classified as `js`
        // by upstream svelte-check (same backend).
        source: DiagnosticSource::Js,
        // tsgo doesn't supply doc URLs in its compact output; we'd
        // need a static lookup table per error code to fill these in
        // (typescript.tv has them but mapping isn't 1-to-1). Leave
        // empty for now — IDEs that want links can derive them from
        // the numeric code.
        code_description_url: None,
    }
}

/// Translate an overlay line into a source line via the line map.
///
/// The map is sorted by overlay_start_line. For an overlay line that
/// falls inside an entry's range, return the corresponding source line
/// preserving the relative offset. For lines OUTSIDE any entry (header
/// comment, function wrapper, void block) return the source line of the
/// nearest preceding entry, or 1 if none exists. This puts synthesized
/// diagnostics on the first line of the most relevant verbatim region
/// rather than at meaningless positions.
fn translate_line(map: &[LineMapEntry], overlay_line: u32) -> Option<u32> {
    if map.is_empty() {
        return None;
    }
    // Find the entry containing overlay_line.
    for entry in map {
        if overlay_line >= entry.overlay_start_line && overlay_line < entry.overlay_end_line {
            let delta = overlay_line - entry.overlay_start_line;
            return Some(entry.source_start_line + delta);
        }
    }
    // Outside any range — clamp to nearest preceding entry.
    let mut nearest_source = 1u32;
    for entry in map {
        if entry.overlay_start_line <= overlay_line {
            nearest_source = entry.source_start_line;
        }
    }
    Some(nearest_source)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn line_maps_for(
        path: &str,
        entries: Vec<LineMapEntry>,
    ) -> HashMap<PathBuf, Vec<LineMapEntry>> {
        let mut m = HashMap::new();
        m.insert(PathBuf::from(path), entries);
        m
    }

    #[test]
    fn maps_generated_file_back_to_source_via_line_map() {
        let layout = CacheLayout::for_workspace("/proj");
        let gen_path = "/proj/.svelte-check/svelte/src/++Foo.svelte.ts";
        // overlay lines 5..15 map to source lines 1..11.
        let map = line_maps_for(
            gen_path,
            vec![LineMapEntry {
                overlay_start_line: 5,
                overlay_end_line: 15,
                source_start_line: 1,
            }],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 10,
            column: 5,
            severity: Severity::Error,
            code: 2322,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &map);
        assert_eq!(mapped.source_path, PathBuf::from("/proj/src/Foo.svelte"));
        // overlay line 10 - overlay_start (5) = 5, + source_start (1) = 6.
        assert_eq!(mapped.line, 6);
    }

    #[test]
    fn passes_through_non_generated_files() {
        let layout = CacheLayout::for_workspace("/proj");
        let raw = RawDiagnostic {
            file: PathBuf::from("/proj/src/plain.ts"),
            line: 4,
            column: 1,
            severity: Severity::Error,
            code: 1000,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &HashMap::new());
        assert_eq!(mapped.source_path, PathBuf::from("/proj/src/plain.ts"));
        assert_eq!(mapped.line, 4); // no offset applied to non-generated files
    }

    #[test]
    fn diagnostics_outside_any_mapped_range_clamp_to_nearest_preceding_source() {
        // Synthesized lines (header, function wrapper, void block) have
        // no exact source location. We place them at the source-start
        // line of the nearest preceding mapped range, defaulting to 1
        // when none precedes.
        let gen_path = "/proj/.svelte-check/svelte/++X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![LineMapEntry {
                overlay_start_line: 10,
                overlay_end_line: 20,
                source_start_line: 5,
            }],
        );
        // Before any mapped range — clamps to source line 1.
        let raw_before = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 1,
            column: 1,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        assert_eq!(map_diagnostic(raw_before, &layout, &map).line, 1);
        // After all mapped ranges — clamps to source line of last entry (5).
        let raw_after = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 30,
            column: 1,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        assert_eq!(map_diagnostic(raw_after, &layout, &map).line, 5);
    }

    #[test]
    fn maps_between_multiple_mapped_ranges_uses_matching_range() {
        // Three contiguous source regions map to three overlay regions.
        // A diagnostic inside the second overlay range maps through the
        // second range's source-start, not the first or third.
        let gen_path = "/proj/.svelte-check/svelte/++X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![
                LineMapEntry {
                    overlay_start_line: 1,
                    overlay_end_line: 3,
                    source_start_line: 1,
                },
                LineMapEntry {
                    overlay_start_line: 10,
                    overlay_end_line: 20,
                    source_start_line: 50,
                },
                LineMapEntry {
                    overlay_start_line: 30,
                    overlay_end_line: 40,
                    source_start_line: 100,
                },
            ],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 15,
            column: 0,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        // 15 is in range [10, 20) — offset 5 from overlay_start (10),
        // applied to source_start (50) = source line 55.
        assert_eq!(map_diagnostic(raw, &layout, &map).line, 55);
    }

    #[test]
    fn maps_between_gaps_clamps_to_previous_range() {
        // A diagnostic lands in the gap between mapped ranges — clamps
        // to the source-start of the nearest preceding range.
        let gen_path = "/proj/.svelte-check/svelte/++X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![
                LineMapEntry {
                    overlay_start_line: 1,
                    overlay_end_line: 3,
                    source_start_line: 1,
                },
                LineMapEntry {
                    overlay_start_line: 10,
                    overlay_end_line: 20,
                    source_start_line: 50,
                },
            ],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 5, // in the gap between [1,3) and [10,20)
            column: 0,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        // Preceding range ended at overlay 3 with source_start 1.
        assert_eq!(map_diagnostic(raw, &layout, &map).line, 1);
    }

    #[test]
    fn empty_line_map_passes_line_through() {
        // When a generated file has no line-map entries at all, the
        // mapper falls through to "line stays as tsgo reported it"
        // rather than erroring.
        let gen_path = "/proj/.svelte-check/svelte/src/X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(gen_path, Vec::new());
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 42,
            column: 0,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &map);
        // Path is still mapped back to the .svelte source.
        assert_eq!(mapped.source_path, PathBuf::from("/proj/src/X.svelte"));
        // With no entries to translate the overlay line, the raw line
        // passes through (clamped to >= 1). This preserves tsgo's
        // line for files that went missing from the line-map — better
        // than silently collapsing to 1 and losing the signal.
        assert_eq!(mapped.line, 42);
    }

    #[test]
    fn generated_path_reverse_maps_to_source_even_without_line_map() {
        // The path-reverse mapping is independent of the line map —
        // a file we emitted lives at <cache>/svelte/<rel>/<name>.svelte.ts
        // and reverses to <workspace>/<rel>/<name>.svelte regardless of
        // whether tsgo's diagnostic has a corresponding line-map entry.
        let gen_path = "/proj/.svelte-check/svelte/lib/components/Foo.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 1,
            column: 1,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &HashMap::new());
        assert_eq!(
            mapped.source_path,
            PathBuf::from("/proj/lib/components/Foo.svelte")
        );
    }

    #[test]
    fn column_and_severity_and_code_pass_through_unchanged() {
        // The mapper only rewrites path and line. Column, severity,
        // code, and message must pass through verbatim.
        let gen_path = "/proj/.svelte-check/svelte/src/X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![LineMapEntry {
                overlay_start_line: 5,
                overlay_end_line: 15,
                source_start_line: 1,
            }],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 7,
            column: 42,
            severity: Severity::Warning,
            code: 2345,
            message: "original message".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &map);
        assert_eq!(mapped.column, 42);
        assert_eq!(mapped.severity, Severity::Warning);
        assert!(
            matches!(mapped.code, DiagnosticCode::Numeric(2345)),
            "numeric code should survive the mapper unchanged; got {:?}",
            mapped.code
        );
        assert_eq!(mapped.message, "original message");
    }

    #[test]
    fn cache_layout_generated_path_round_trips_through_reverse() {
        // Round-trip: a .svelte path → generated_path → original_from_generated
        // must return the same .svelte path. This is the invariant the
        // diagnostic mapper depends on, exercised here independent of
        // map_diagnostic itself.
        let layout = CacheLayout::for_workspace("/proj");
        let svelte_path = PathBuf::from("/proj/src/lib/components/Button.svelte");
        let generated = layout.generated_path(&svelte_path);
        let back = layout
            .original_from_generated(&generated)
            .expect("reverse mapping must succeed for a path we generated");
        assert_eq!(back, svelte_path);
    }

    #[test]
    fn cache_layout_reverse_rejects_paths_outside_svelte_dir() {
        // Paths that don't live under <cache>/svelte/ aren't ours and
        // should reverse to None — the mapper then skips the rewrite
        // and passes the path through as-is.
        let layout = CacheLayout::for_workspace("/proj");
        assert!(
            layout
                .original_from_generated(Path::new("/proj/src/plain.ts"))
                .is_none()
        );
        assert!(
            layout
                .original_from_generated(Path::new("/elsewhere/X.ts"))
                .is_none()
        );
    }
}
