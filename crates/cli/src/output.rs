//! Diagnostic output formatting.
//!
//! Four output formats are supported, mirroring upstream svelte-check:
//!
//! - `machine` — line-oriented `<ts> <TYPE> "<file>" <line>:<col> "<msg>"`,
//!   one diagnostic per line, ending with a `COMPLETED` line.
//! - `machine-verbose` — same shape, but each diagnostic is a JSON
//!   object on its own line. Used by editor / CI integrations that
//!   want richer payloads.
//! - `human` — terse, color-by-default `path:line:col\n<Severity>: <msg>`.
//! - `human-verbose` (default) — adds a "loading" banner and a code
//!   frame under each diagnostic.
//!
//! Color, ANSI escapes, and the `COMPLETED` denominator are all
//! formatted identically to upstream so existing wrappers parsing the
//! prelude / completion lines keep working.

use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::ColorMode;

/// Unwrap a stdout write result, preserving `println!`'s
/// panic-on-write-failure posture (same message shape as std's
/// `print_to` panic). The pre-buffering code used `println!` per line
/// (one lock + write syscall each — measured 0.6-0.8s of overhead at
/// 40k human-verbose diagnostics); swallowing a write error after the
/// switch to an explicit writer would silently truncate machine
/// output that CI parsers key off, so a failed write still panics
/// exactly as `println!` did.
fn stdout_write_ok(result: std::io::Result<()>) {
    if let Err(err) = result {
        panic!("failed printing to stdout: {err}");
    }
}

/// `writeln!` into the locked, buffered stdout writer through
/// [`stdout_write_ok`].
macro_rules! outln {
    ($out:expr) => {
        stdout_write_ok(writeln!($out))
    };
    ($out:expr, $($arg:tt)*) => {
        stdout_write_ok(writeln!($out, $($arg)*))
    };
}

/// Compute `target` relative to `base`, walking up with `..` segments
/// when `target` is not a descendant of `base` (mirrors Node's
/// `path.relative`). Both paths must be absolute and resolved — they are
/// at every callsite here. Finds the common leading components, emits one
/// `..` per remaining `base` component, then appends the remaining
/// `target` components. The in-workspace common case yields the identical
/// descendant-relative path a plain prefix-strip would; the out-of-base
/// case (a `source_path` above the workspace) now produces a correct
/// `../`-prefixed path instead of leaking the absolute path.
fn relative_path(target: &Path, base: &Path) -> PathBuf {
    let t: Vec<Component<'_>> = target.components().collect();
    let b: Vec<Component<'_>> = base.components().collect();
    let common = t.iter().zip(b.iter()).take_while(|(a, c)| a == c).count();
    let mut rel = PathBuf::new();
    for _ in common..b.len() {
        rel.push("..");
    }
    for comp in &t[common..] {
        rel.push(comp.as_os_str());
    }
    rel
}

static SHOWN_WORKSPACE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Record the workspace as the user named it. Upstream prints the path
/// it was given (made absolute, `..` resolved) rather than resolving
/// symlinks, while everything else here works on the canonical path.
pub(crate) fn set_shown_workspace(path: PathBuf) {
    let _ = SHOWN_WORKSPACE.set(path);
}

/// The workspace path to print: the user's spelling when it names
/// `workspace`, else `workspace` itself (e.g. after the run moved to a
/// referenced project).
fn shown_workspace(workspace: &Path) -> &Path {
    match SHOWN_WORKSPACE.get() {
        Some(shown) if dunce::canonicalize(shown).ok().as_deref() == Some(workspace) => shown,
        _ => workspace,
    }
}

/// Whether a diagnostic clears the `--threshold` bar for *display*.
/// `error` shows only errors; `warning` (the default) shows everything.
/// Summary counts are computed independently of this — the threshold is
/// a print-time filter only (mirrors upstream's `diagnosticFilter`).
fn passes_threshold(severity: svn_typecheck::Severity, threshold: &str) -> bool {
    if threshold == "error" {
        matches!(severity, svn_typecheck::Severity::Error)
    } else {
        true
    }
}

pub(crate) fn print_diagnostics(
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    output_format: &str,
    color: ColorMode,
    files_checked: usize,
    threshold: &str,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let errors = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Error))
        .count();
    let warnings = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Warning))
        .count();
    // Upstream counts a file as "with problems" only when it has an
    // Error or Warning — Hint/suggestion-only files don't count
    // (index.ts increments fileCountWithProblems on Error/Warning).
    // Hints reach us only under `--include-suggestions`; without this
    // gate a hint-only file would inflate FILES_WITH_PROBLEMS / the
    // "in N files" phrasing.
    let files_with_problems: std::collections::HashSet<_> = diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.severity,
                svn_typecheck::Severity::Error | svn_typecheck::Severity::Warning
            )
        })
        .map(|d| &d.source_path)
        .collect();
    let use_color = color.use_color();

    // One locked, buffered writer for the whole report. Every helper
    // below writes through it, so a diagnostic-heavy run issues a
    // handful of large write syscalls instead of one per output line
    // (std's unlocked `println!` flushes LineWriter-buffered stdout at
    // every newline).
    let mut out = std::io::BufWriter::new(std::io::stdout().lock());

    match output_format {
        "machine-verbose" => {
            print_machine(&mut out, workspace, diagnostics, now_ms, true, threshold);
            print_machine_completed(
                &mut out,
                now_ms,
                files_checked,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "machine" => {
            print_machine(&mut out, workspace, diagnostics, now_ms, false, threshold);
            print_machine_completed(
                &mut out,
                now_ms,
                files_checked,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "human" => {
            print_human(
                &mut out,
                workspace,
                diagnostics,
                false,
                use_color,
                threshold,
            );
            print_human_summary(
                &mut out,
                errors,
                warnings,
                files_with_problems.len(),
                use_color,
            );
        }
        // human-verbose is the default
        _ => {
            // Verbose mode prints a banner before diagnostics — matches
            // upstream svelte-check so editor integrations and shell
            // wrappers parsing the prelude don't break.
            outln!(
                out,
                "Loading svelte-check in workspace: {}",
                shown_workspace(workspace).display()
            );
            outln!(out, "Getting Svelte diagnostics...");
            outln!(out);
            print_human(&mut out, workspace, diagnostics, true, use_color, threshold);
            print_human_summary(
                &mut out,
                errors,
                warnings,
                files_with_problems.len(),
                use_color,
            );
        }
    }
    // Flush before the writer drops so a failure surfaces as the same
    // loud panic a failed `println!` produced (BufWriter's Drop flush
    // swallows errors).
    stdout_write_ok(out.flush());
}

/// `machine` and `machine-verbose` body — per-diagnostic lines.
fn print_machine(
    out: &mut impl Write,
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    now_ms: u128,
    verbose: bool,
    threshold: &str,
) {
    // JSON-escape the workspace path (mirrors upstream's
    // `START ${JSON.stringify(workspaceDir)}`) so a path containing a
    // quote or backslash doesn't break machine-output parsers.
    let shown = shown_workspace(workspace);
    let ws = serde_json::to_string(&shown.display().to_string())
        .unwrap_or_else(|_| format!("\"{}\"", shown.display()));
    outln!(out, "{now_ms} START {ws}");
    for d in diagnostics {
        if !passes_threshold(d.severity, threshold) {
            continue;
        }
        let rel = relative_path(&d.source_path, workspace);
        let type_label = match d.severity {
            svn_typecheck::Severity::Error => "ERROR",
            svn_typecheck::Severity::Warning => "WARNING",
            svn_typecheck::Severity::Hint => "HINT",
        };
        if verbose {
            // Build the payload field-by-field so the `code` value
            // serializes as a number for TS diagnostics and as a
            // quoted string for compiler diagnostics — matches
            // upstream svelte-check's machine-verbose output. Same
            // story for `codeDescription`: only present when we have
            // a documentation URL.
            let mut obj = serde_json::Map::new();
            obj.insert("type".to_string(), serde_json::json!(type_label));
            obj.insert(
                "filename".to_string(),
                serde_json::json!(rel.to_string_lossy()),
            );
            obj.insert(
                "start".to_string(),
                serde_json::json!({
                    "line": d.line.saturating_sub(1),
                    "character": d.column.saturating_sub(1),
                }),
            );
            obj.insert(
                "end".to_string(),
                serde_json::json!({
                    "line": d.end_line.saturating_sub(1),
                    "character": d.end_column.saturating_sub(1),
                }),
            );
            obj.insert("message".to_string(), serde_json::json!(d.message));
            // A code-less diagnostic has no `code` key at all, as
            // `JSON.stringify` drops an undefined field.
            match &d.code {
                svn_typecheck::DiagnosticCode::Numeric(n) => {
                    obj.insert("code".to_string(), serde_json::json!(n));
                }
                svn_typecheck::DiagnosticCode::Slug(s) => {
                    obj.insert("code".to_string(), serde_json::json!(s));
                }
                svn_typecheck::DiagnosticCode::Missing => {}
            }
            if let Some(href) = &d.code_description_url {
                obj.insert(
                    "codeDescription".to_string(),
                    serde_json::json!({ "href": href }),
                );
            }
            obj.insert("source".to_string(), serde_json::json!(d.source.as_str()));
            let payload = serde_json::Value::Object(obj);
            outln!(out, "{now_ms} {payload}");
        } else {
            // Non-verbose: line-oriented `<ts> <TYPE> "<file>" <line>:<col> "<msg>"`.
            let fname = serde_json::to_string(&rel.to_string_lossy()).unwrap_or_default();
            let msg = serde_json::to_string(&d.message).unwrap_or_default();
            outln!(
                out,
                "{now_ms} {type_label} {fname} {}:{} {msg}",
                d.line,
                d.column,
            );
        }
    }
}

fn print_machine_completed(
    out: &mut impl Write,
    now_ms: u128,
    files_checked: usize,
    errors: usize,
    warnings: usize,
    files_with_problems: usize,
) {
    outln!(
        out,
        "{now_ms} COMPLETED {files_checked} FILES {errors} ERRORS {warnings} WARNINGS {files_with_problems} FILES_WITH_PROBLEMS"
    );
}

/// Emit a machine-output `FAILURE` line for a fatal check error,
/// mirroring upstream's `MachineFriendlyWriter.failure`
/// (`FAILURE ${JSON.stringify(err.message)}`). Machine consumers key
/// off this line; without it a crash looks like a silent stop. No-op
/// for human formats (the caller prints to stderr there).
pub(crate) fn print_machine_failure(output_format: &str, message: &str) {
    if output_format != "machine" && output_format != "machine-verbose" {
        return;
    }
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let msg = serde_json::to_string(message).unwrap_or_else(|_| format!("\"{message}\""));
    let mut out = std::io::stdout().lock();
    outln!(out, "{now_ms} FAILURE {msg}");
}

/// `human` / `human-verbose` body — per-diagnostic block.
fn print_human(
    out: &mut impl Write,
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    verbose: bool,
    color: bool,
    threshold: &str,
) {
    let workspace_display = shown_workspace(workspace).display().to_string();
    // Code-frame source memo. Diagnostics arrive grouped by file, so a
    // one-entry cache turns N-diagnostics-per-file into one read per
    // distinct file instead of one full read + line-split per
    // diagnostic. `None` content records a failed read so we don't
    // retry it per diagnostic either.
    let mut frame_source: Option<(&Path, Option<String>)> = None;
    for d in diagnostics {
        if !passes_threshold(d.severity, threshold) {
            continue;
        }
        let rel = relative_path(&d.source_path, workspace);
        let filename = rel.display().to_string();
        // Path that IDEs turn into clickable links. Use the platform
        // separator between workspace and file (upstream uses `sep`), so
        // Windows links resolve.
        outln!(
            out,
            "{workspace_display}{}{}:{}:{}",
            std::path::MAIN_SEPARATOR,
            paint(&filename, GREEN, color),
            d.line,
            d.column,
        );
        // `(svelte)` / `(js)` / `(ts)` / `(css)`, as upstream prints it.
        let source = d.source.as_str();
        let label = match d.severity {
            svn_typecheck::Severity::Error => paint("Error", RED, color),
            svn_typecheck::Severity::Warning => paint("Warn", YELLOW, color),
            // Upstream prints a hint's location and nothing else.
            svn_typecheck::Severity::Hint => {
                outln!(out);
                continue;
            }
        };
        if verbose {
            if frame_source
                .as_ref()
                .is_none_or(|(p, _)| *p != d.source_path.as_path())
            {
                frame_source = Some((
                    d.source_path.as_path(),
                    std::fs::read_to_string(&d.source_path).ok(),
                ));
            }
            let frame = match &frame_source {
                Some((_, Some(text))) => code_frame(text, d, color),
                _ => String::new(),
            };
            outln!(
                out,
                "{label}: {} ({source})\n{}",
                d.message,
                paint(frame.trim_end(), CYAN, color),
            );
        } else {
            outln!(out, "{label}: {} ({source})", d.message);
        }
        outln!(out);
    }
}

fn print_human_summary(
    out: &mut impl Write,
    errors: usize,
    warnings: usize,
    files: usize,
    color: bool,
) {
    // Mirror upstream's completion line (writers.ts): a `====` rule
    // when any file has problems, then `svelte-check-native found …`
    // (we brand the human summary; machine output stays byte-compatible
    // with upstream for editor integrations), with the
    // `in N files` clause present ONLY when files > 0 and NO elapsed
    // time (use `--timings` for that). Pre-fix we always printed
    // `in N files` and an elapsed suffix upstream doesn't emit.
    if files > 0 {
        outln!(out, "====================================");
    }
    let in_files = if files > 0 {
        format!(" in {files} file{}", if files == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    // The trailing newline sits inside the colour codes upstream.
    let parts = format!(
        "svelte-check-native found {} error{} and {} warning{}{in_files}\n",
        errors,
        if errors == 1 { "" } else { "s" },
        warnings,
        if warnings == 1 { "" } else { "s" },
    );
    let tint = if errors > 0 {
        RED
    } else if warnings > 0 {
        YELLOW
    } else {
        GREEN
    };
    let _ = write!(out, "{}", paint(&parts, tint, color));
}

const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const MAGENTA: &str = "\x1b[35m";
const CYAN: &str = "\x1b[36m";
/// picocolors closes every foreground colour with this code.
const FG_CLOSE: &str = "\x1b[39m";

/// picocolors' `formatter(open, close)`: wrap `text`, re-opening the
/// colour wherever an inner colour closed, so nested colours survive.
fn paint(text: &str, open: &str, color: bool) -> String {
    if !color {
        return text.to_string();
    }
    let mut out = String::with_capacity(text.len() + 10);
    out.push_str(open);
    // The search starts `open.len()` bytes in, as picocolors' does.
    match text
        .get(open.len()..)
        .and_then(|rest| rest.find(FG_CLOSE))
        .map(|i| i + open.len())
    {
        Some(first) => {
            out.push_str(&text[..first]);
            out.push_str(&text[first..].replace(FG_CLOSE, open));
        }
        None => out.push_str(text),
    }
    out.push_str(FG_CLOSE);
    out
}

/// Upstream's `formatRelatedCode`: the line before the diagnostic, the
/// diagnostic's lines with its range highlighted, and the line after,
/// each with its line break. Positions follow `offsetAt`: a line past
/// the end reads as empty, a column past the line end stops at the
/// line break.
fn code_frame(text: &str, d: &svn_typecheck::CheckDiagnostic, color: bool) -> String {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(i, _)| i + 1))
        .collect();
    // `offsetAt({ line, character })` with 0-based line and a UTF-16
    // character count.
    let offset_at = |line: i64, character: usize| -> usize {
        if line < 0 {
            return 0;
        }
        let line = line as usize;
        let Some(&start) = line_starts.get(line) else {
            return text.len();
        };
        let next = line_starts.get(line + 1).copied().unwrap_or(text.len());
        let mut units = 0;
        for (i, ch) in text[start..next].char_indices() {
            if units >= character {
                return start + i;
            }
            units += ch.len_utf16();
        }
        next
    };
    let start_line = i64::from(d.line) - 1;
    let end_line = i64::from(d.end_line) - 1;
    let start = offset_at(start_line, d.column.saturating_sub(1) as usize);
    let end = offset_at(end_line, d.end_column.saturating_sub(1) as usize).max(start);
    let line_text = |line: i64| &text[offset_at(line, 0)..offset_at(line, usize::MAX)];
    let mut frame = String::new();
    frame.push_str(line_text(start_line - 1));
    frame.push_str(&text[offset_at(start_line, 0)..start]);
    frame.push_str(&paint(&text[start..end], MAGENTA, color));
    frame.push_str(&text[end..offset_at(end_line, usize::MAX)]);
    frame.push_str(line_text(end_line + 1));
    frame
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    fn diag(
        line: u32,
        column: u32,
        end_line: u32,
        end_column: u32,
    ) -> svn_typecheck::CheckDiagnostic {
        svn_typecheck::CheckDiagnostic {
            source_path: PathBuf::from("/w/A.svelte"),
            line,
            column,
            end_line,
            end_column,
            severity: svn_typecheck::Severity::Error,
            code: svn_typecheck::DiagnosticCode::Numeric(2322),
            message: String::new(),
            source: svn_typecheck::DiagnosticSource::Ts,
            code_description_url: None,
        }
    }

    #[test]
    fn code_frame_is_the_surrounding_lines_verbatim() {
        let src = "one\n\tlet x = 1;\nthree\nfour\n";
        assert_eq!(
            code_frame(src, &diag(2, 6, 2, 7), false),
            "one\n\tlet x = 1;\nthree\n"
        );
    }

    #[test]
    fn code_frame_highlights_the_range_in_magenta() {
        let src = "a\nbcd\n";
        assert_eq!(
            code_frame(src, &diag(2, 2, 2, 3), true),
            "a\nb\x1b[35mc\x1b[39md\n"
        );
    }

    #[test]
    fn code_frame_at_the_edges_reads_missing_lines_as_empty() {
        let src = "only";
        assert_eq!(code_frame(src, &diag(1, 1, 1, 5), false), "only");
    }

    #[test]
    fn nested_colours_reopen_the_outer_colour() {
        assert_eq!(
            paint("x\x1b[35my\x1b[39mz", CYAN, true),
            "\x1b[36mx\x1b[35my\x1b[36mz\x1b[39m"
        );
    }

    #[test]
    fn relative_path_descendant_matches_strip_prefix() {
        let base = Path::new("/home/user/project");
        let target = Path::new("/home/user/project/src/App.svelte");
        assert_eq!(relative_path(target, base), Path::new("src/App.svelte"),);
    }

    #[test]
    fn relative_path_above_workspace_walks_up() {
        // A `source_path` above the workspace must produce a
        // `../`-prefixed path, not the leaked absolute path.
        let base = Path::new("/home/user/project/app");
        let target = Path::new("/home/user/project/shared/Lib.svelte");
        assert_eq!(
            relative_path(target, base),
            Path::new("../shared/Lib.svelte"),
        );
    }
}
