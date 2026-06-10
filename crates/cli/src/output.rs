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

use std::path::Path;

use crate::ColorMode;

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

    match output_format {
        "machine-verbose" => {
            print_machine(workspace, diagnostics, now_ms, true, threshold);
            print_machine_completed(
                now_ms,
                files_checked,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "machine" => {
            print_machine(workspace, diagnostics, now_ms, false, threshold);
            print_machine_completed(
                now_ms,
                files_checked,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "human" => {
            print_human(workspace, diagnostics, false, use_color, threshold);
            print_human_summary(errors, warnings, files_with_problems.len(), use_color);
        }
        // human-verbose is the default
        _ => {
            // Verbose mode prints a banner before diagnostics — matches
            // upstream svelte-check so editor integrations and shell
            // wrappers parsing the prelude don't break.
            println!("Loading svelte-check in workspace: {}", workspace.display());
            println!("Getting Svelte diagnostics...");
            println!();
            print_human(workspace, diagnostics, true, use_color, threshold);
            print_human_summary(errors, warnings, files_with_problems.len(), use_color);
        }
    }
}

/// `machine` and `machine-verbose` body — per-diagnostic lines.
fn print_machine(
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    now_ms: u128,
    verbose: bool,
    threshold: &str,
) {
    // JSON-escape the workspace path (mirrors upstream's
    // `START ${JSON.stringify(workspaceDir)}`) so a path containing a
    // quote or backslash doesn't break machine-output parsers.
    let ws = serde_json::to_string(&workspace.display().to_string())
        .unwrap_or_else(|_| format!("\"{}\"", workspace.display()));
    println!("{now_ms} START {ws}");
    for d in diagnostics {
        if !passes_threshold(d.severity, threshold) {
            continue;
        }
        let rel = d
            .source_path
            .strip_prefix(workspace)
            .unwrap_or(&d.source_path);
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
            obj.insert(
                "code".to_string(),
                match &d.code {
                    svn_typecheck::DiagnosticCode::Numeric(n) => serde_json::json!(n),
                    svn_typecheck::DiagnosticCode::Slug(s) => serde_json::json!(s),
                },
            );
            if let Some(href) = &d.code_description_url {
                obj.insert(
                    "codeDescription".to_string(),
                    serde_json::json!({ "href": href }),
                );
            }
            obj.insert("source".to_string(), serde_json::json!(d.source.as_str()));
            let payload = serde_json::Value::Object(obj);
            println!("{now_ms} {payload}");
        } else {
            // Non-verbose: line-oriented `<ts> <TYPE> "<file>" <line>:<col> "<msg>"`.
            let fname = serde_json::to_string(&rel.to_string_lossy()).unwrap_or_default();
            let msg = serde_json::to_string(&d.message).unwrap_or_default();
            println!(
                "{now_ms} {type_label} {fname} {}:{} {msg}",
                d.line, d.column,
            );
        }
    }
}

fn print_machine_completed(
    now_ms: u128,
    files_checked: usize,
    errors: usize,
    warnings: usize,
    files_with_problems: usize,
) {
    println!(
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
    println!("{now_ms} FAILURE {msg}");
}

/// `human` / `human-verbose` body — per-diagnostic block.
fn print_human(
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    verbose: bool,
    color: bool,
    threshold: &str,
) {
    let workspace_display = workspace.display().to_string();
    for d in diagnostics {
        if !passes_threshold(d.severity, threshold) {
            continue;
        }
        let rel = d
            .source_path
            .strip_prefix(workspace)
            .unwrap_or(&d.source_path);
        let filename = rel.display().to_string();
        // Path that IDEs turn into clickable links. Use the platform
        // separator between workspace and file (upstream uses `sep`), so
        // Windows links resolve.
        println!(
            "{workspace_display}{}{}:{}:{}",
            std::path::MAIN_SEPARATOR,
            paint(&filename, "32", color),
            d.line,
            d.column,
        );
        let label = match d.severity {
            svn_typecheck::Severity::Error => paint("Error", "31", color),
            svn_typecheck::Severity::Warning => paint("Warn", "33", color),
            svn_typecheck::Severity::Hint => paint("Hint", "36", color),
        };
        // Span length for the code-frame caret. We have a real
        // [start, end) so prefer that; fall back to 1 char when the
        // span is empty (zero-width markers still get visualized).
        let span = d.end_column.saturating_sub(d.column);
        let span = if span == 0 { Some(1) } else { Some(span) };
        // Upstream prints the diagnostic SOURCE (`(svelte)` / `(js)` /
        // `(css)`), not the code — `${message} (${diagnostic.source})`.
        let source = d.source.as_str();
        if verbose {
            // Code frame: try to read the source file and emit a short
            // excerpt around the diagnostic line, with a caret pointer.
            let frame = format_code_frame(&d.source_path, d.line, d.column, span);
            if frame.is_empty() {
                println!("{label}: {} ({source})", d.message);
            } else {
                println!(
                    "{label}: {} ({source})\n{}",
                    d.message,
                    paint(&frame, "36", color),
                );
            }
        } else {
            println!("{label}: {} ({source})", d.message);
        }
        println!();
    }
}

fn print_human_summary(errors: usize, warnings: usize, files: usize, color: bool) {
    // Mirror upstream's completion line (writers.ts): a `====` rule
    // when any file has problems, then `svelte-check found …`, with the
    // `in N files` clause present ONLY when files > 0 and NO elapsed
    // time (use `--timings` for that). Pre-fix we always printed
    // `in N files` and an elapsed suffix upstream doesn't emit.
    if files > 0 {
        println!("====================================");
    }
    let in_files = if files > 0 {
        format!(" in {files} file{}", if files == 1 { "" } else { "s" })
    } else {
        String::new()
    };
    let parts = format!(
        "svelte-check found {} error{} and {} warning{}{in_files}",
        errors,
        if errors == 1 { "" } else { "s" },
        warnings,
        if warnings == 1 { "" } else { "s" },
    );
    if errors > 0 {
        println!("{}", paint(&parts, "31", color));
    } else if warnings > 0 {
        println!("{}", paint(&parts, "33", color));
    } else {
        println!("{}", paint(&parts, "32", color));
    }
}

/// Wrap `text` in an ANSI color code if `color` is true. Cheap fallback to
/// plain text when stdout isn't a terminal.
fn paint(text: &str, code: &str, color: bool) -> String {
    if color {
        format!("\x1b[{code}m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Read the source file and produce a short code frame around the
/// (1-based) diagnostic line. Returns an empty string on read failure or
/// out-of-range line numbers — caller falls back to no-frame output.
fn format_code_frame(path: &Path, line: u32, column: u32, span_length: Option<u32>) -> String {
    let Ok(source) = std::fs::read_to_string(path) else {
        return String::new();
    };
    render_code_frame(&source, line, column, span_length)
}

/// Pure code-frame renderer — split out for testability. Takes the whole
/// file's text plus a 1-based (line, column) and produces a 3-line frame
/// with the target line highlighted by a `^^^` caret underneath.
///
/// Tab handling: the source line is printed verbatim (tabs preserved),
/// and the caret line mirrors the source's whitespace through column-1
/// — writing a tab where the source had a tab, space elsewhere. The
/// terminal's own tab expansion then aligns both lines to the same
/// visual column regardless of the configured tab width. Without this,
/// tab-indented files render with the caret several visual columns
/// left of the actual error site (filed by a user with a Svelte project
/// whose indent was tabs: `bind:value={addAssemblyPrice}` fired TS2322
/// but the caret appeared under `type="number"` on the line above).
fn render_code_frame(source: &str, line: u32, column: u32, span_length: Option<u32>) -> String {
    let lines: Vec<&str> = source.lines().collect();
    let target_idx = match (line as usize).checked_sub(1) {
        Some(i) if i < lines.len() => i,
        _ => return String::new(),
    };
    let start = target_idx.saturating_sub(1);
    let end = (target_idx + 2).min(lines.len());
    let mut out = String::new();
    let width = (end).to_string().len();
    for (i, &content) in lines[start..end].iter().enumerate() {
        let ln = start + i + 1;
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{ln:>width$} | {content}\n"));
        if ln == line as usize {
            // Gutter: "<ln> | " — `width` digits + space + pipe + space.
            for _ in 0..(width + 3) {
                out.push(' ');
            }
            // Preserve each whitespace kind from the source line up to
            // the error column so terminal tab-expansion aligns caret
            // and source identically. Non-whitespace chars before the
            // column (rare for error sites but possible for multi-byte
            // identifiers etc.) still get a single space — sufficient
            // for caret counting since `column` is 1-based char index.
            let column_idx = column.saturating_sub(1) as usize;
            for ch in content.chars().take(column_idx) {
                out.push(if ch == '\t' { '\t' } else { ' ' });
            }
            let underline = "^".repeat(span_length.unwrap_or(1).max(1) as usize);
            out.push_str(&underline);
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    /// Return the caret line from a rendered code frame — the line that
    /// starts at the gutter padding and contains the `^` underline.
    fn extract_caret_line(frame: &str) -> &str {
        frame
            .lines()
            .find(|l| l.trim_start().starts_with('^'))
            .unwrap_or("")
    }

    /// Return the target source line from a rendered frame — the one with
    /// gutter `N | ` matching the requested line number.
    fn extract_source_line(frame: &str, line: u32) -> &str {
        let prefix = format!("{line} | ");
        // Also tolerate right-aligned gutters (`"  3 | "`): strip leading
        // spaces before comparing.
        frame
            .lines()
            .find(|l| l.trim_start().starts_with(&prefix))
            .unwrap_or("")
    }

    #[test]
    fn code_frame_caret_aligns_with_error_on_tab_indented_source() {
        // Regression: a user reported that on Windows, a tab-indented
        // file showed the `^^^` caret several visual columns left of
        // the actual error site. Root cause was spaces-only caret
        // padding while the source line rendered its tabs verbatim —
        // terminal tab-expansion made the source wider than the caret
        // counted for. Fix is to mirror the source's whitespace kind.
        let src = "line one\n\t\t\tbind:value={x}\nline three\n";
        // `bind:value={x}` starts at char column 4 (3 tabs + 1-based).
        let frame = render_code_frame(src, 2, 4, Some(14));
        let src_line = extract_source_line(&frame, 2);
        let caret_line = extract_caret_line(&frame);

        // After the gutter, the caret prefix must contain exactly the
        // same TABS as the source line before the error column.
        let src_prefix_tabs = src_line.chars().filter(|&c| c == '\t').count();
        let caret_prefix_tabs = caret_line.chars().filter(|&c| c == '\t').count();
        assert_eq!(
            src_prefix_tabs, caret_prefix_tabs,
            "caret line must mirror source tabs so terminal expansion aligns them\n\
             frame:\n{frame}",
        );
        assert!(
            caret_line.contains("^^^^^^^^^^^^^^"),
            "14-char underline missing; frame:\n{frame}",
        );
    }

    #[test]
    fn code_frame_caret_uses_spaces_on_space_indented_source() {
        // Sanity: space-indented source must still produce a
        // space-only caret prefix (no tabs sneaking in).
        let src = "line one\n    bind:value={x}\nline three\n";
        let frame = render_code_frame(src, 2, 5, Some(14));
        let caret_line = extract_caret_line(&frame);
        assert!(
            !caret_line.contains('\t'),
            "caret line must not contain tabs when source is space-indented\nframe:\n{frame}",
        );
        assert!(caret_line.contains("^^^^^^^^^^^^^^"), "frame:\n{frame}");
    }

    #[test]
    fn code_frame_returns_empty_for_line_out_of_range() {
        let src = "only one line\n";
        assert_eq!(render_code_frame(src, 5, 1, Some(1)), "");
    }

    #[test]
    fn code_frame_handles_first_line_with_no_preceding_line() {
        // No `line - 1` context available; we still emit the target
        // line + any trailing context.
        let src = "first line\nsecond line\n";
        let frame = render_code_frame(src, 1, 1, Some(5));
        assert!(
            frame.contains("first line"),
            "target line missing from frame:\n{frame}",
        );
    }
}
