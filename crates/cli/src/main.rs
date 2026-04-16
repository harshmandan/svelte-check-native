//! `svelte-check-native` — CLI entrypoint.
//!
//! Phase 1: minimum useful command surface.
//!
//! Currently supports `--emit-ts` (debug print of generated TypeScript) and
//! accepts the broader flag set required by the bug-fixtures and
//! upstream-sanity test runners. Type-checking diagnostics (the actual
//! purpose of the binary) land once the emit + analyze + typecheck crates
//! are wired together.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use walkdir::WalkDir;

#[derive(Parser, Debug)]
#[command(
    name = "svelte-check-native",
    version,
    about = "CLI-only type checker for Svelte 5+ projects. Powered by tsgo.",
    long_about = "svelte-check-native — type-check Svelte 5+ components.\n\n\
                  Svelte 4 syntax (export let, $:, <slot>, on:event) is not\n\
                  supported. tsgo (@typescript/native-preview) must be installed\n\
                  in the project's node_modules, or pointed at via TSGO_BIN."
)]
struct Cli {
    /// Workspace root to scan. Defaults to current working directory.
    #[arg(long)]
    workspace: Option<PathBuf>,

    /// Path to tsconfig.json (or jsconfig.json). When omitted, walks up from
    /// `--workspace` looking for one.
    #[arg(long)]
    tsconfig: Option<PathBuf>,

    /// Disable tsconfig discovery; only Svelte-only diagnostics.
    #[arg(long = "no-tsconfig", default_value_t = false)]
    no_tsconfig: bool,

    /// Output format. Accepted values match upstream svelte-check.
    #[arg(long, default_value = "human-verbose")]
    output: String,

    /// Diagnostic-source filter: comma-separated subset of
    /// `ts,js,svelte,css`.
    #[arg(long = "diagnostic-sources")]
    diagnostic_sources: Option<String>,

    /// Diagnostic threshold: `warning` (show all) or `error` (errors only).
    #[arg(long, default_value = "warning")]
    threshold: String,

    /// Exit non-zero on warnings.
    #[arg(long = "fail-on-warnings", default_value_t = false)]
    fail_on_warnings: bool,

    /// Compiler-warning severity overrides (`code:severity,code:severity`).
    #[arg(long = "compiler-warnings")]
    compiler_warnings: Option<String>,

    /// Comma-separated globs to ignore. Only valid with `--no-tsconfig`.
    #[arg(long)]
    ignore: Option<String>,

    /// Enable disk caching. No-op for us — caching is always on; accepted
    /// for upstream-compat.
    #[arg(long, default_value_t = false)]
    incremental: bool,

    /// Use tsgo. No-op for us — tsgo is always on; accepted for
    /// upstream-compat.
    #[arg(long, default_value_t = false)]
    tsgo: bool,

    /// Force ANSI colors.
    #[arg(long, default_value_t = false)]
    color: bool,

    /// Force no ANSI colors.
    #[arg(long = "no-color", default_value_t = false)]
    no_color: bool,

    /// Print generated TypeScript for each Svelte file (debug).
    #[arg(long = "emit-ts", default_value_t = false)]
    emit_ts: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // CLAUDECODE=1 → force machine output (matches upstream svelte-check).
    let output = if std::env::var("CLAUDECODE").as_deref() == Ok("1") {
        "machine".to_string()
    } else {
        cli.output.clone()
    };

    let workspace_arg = cli
        .workspace
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    // Canonicalize so subsequent walk-up logic (tsgo discovery, tsconfig
    // search) traverses real filesystem ancestors. Without this, a relative
    // workspace like `./test-success` walks `.parent()` → `./` → `""` →
    // None and never reaches actual node_modules locations.
    let workspace = match workspace_arg.canonicalize() {
        Ok(p) => p,
        Err(err) => {
            eprintln!(
                "svelte-check-native: cannot resolve workspace {}: {err}",
                workspace_arg.display()
            );
            return ExitCode::from(2);
        }
    };

    if cli.emit_ts {
        return run_emit_ts(&workspace);
    }

    let tsconfig = match resolve_tsconfig(&workspace, cli.tsconfig.as_deref(), cli.no_tsconfig) {
        Ok(Some(p)) => Some(p),
        Ok(None) => None,
        Err(msg) => {
            eprintln!("svelte-check-native: {msg}");
            return ExitCode::from(2);
        }
    };

    let Some(tsconfig) = tsconfig else {
        eprintln!(
            "svelte-check-native: --no-tsconfig mode is not yet implemented; pass --tsconfig <path> or run inside a project with a tsconfig.json"
        );
        return ExitCode::from(2);
    };

    run_typecheck(
        &workspace,
        &tsconfig,
        &output,
        &cli.threshold,
        cli.fail_on_warnings,
    )
}

/// Resolve the user's tsconfig path. Honors `--tsconfig`, `--no-tsconfig`,
/// and otherwise walks up from the workspace looking for `tsconfig.json`
/// then `jsconfig.json`.
fn resolve_tsconfig(
    workspace: &Path,
    explicit: Option<&Path>,
    no_tsconfig: bool,
) -> Result<Option<PathBuf>, String> {
    if no_tsconfig {
        return Ok(None);
    }
    if let Some(p) = explicit {
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            workspace.join(p)
        };
        if !resolved.is_file() {
            return Err(format!("tsconfig not found at {}", resolved.display()));
        }
        // Canonicalize so the overlay's `extends` path is computable as a
        // proper relative path between two absolute directories.
        return Ok(Some(resolved.canonicalize().unwrap_or(resolved)));
    }
    let mut cur: Option<&Path> = Some(workspace);
    while let Some(dir) = cur {
        for name in ["tsconfig.json", "jsconfig.json"] {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(Some(candidate));
            }
        }
        cur = dir.parent();
    }
    Err(format!(
        "no tsconfig.json or jsconfig.json found at or above {}",
        workspace.display()
    ))
}

/// Default flow: parse + emit each .svelte file, hand the lot to tsgo,
/// format diagnostics, exit with the appropriate code.
///
/// `threshold` controls which diagnostics are kept: `error` filters out
/// warnings; `warning` keeps both. `fail_on_warnings` makes warnings
/// participate in the exit-code decision (matching upstream
/// svelte-check).
fn run_typecheck(
    workspace: &Path,
    tsconfig: &Path,
    output_format: &str,
    threshold: &str,
    fail_on_warnings: bool,
) -> ExitCode {
    let svelte_files = discover_svelte_files(workspace);

    let mut inputs: Vec<svn_typecheck::CheckInput> = Vec::with_capacity(svelte_files.len());
    for file in &svelte_files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("failed to read {}: {err}", file.display());
                continue;
            }
        };
        let (doc, _parse_errors) = svn_parser::parse_sections(&source);
        let (fragment, _template_errors) =
            svn_parser::parse_all_template_runs(&source, &doc.template.text_runs);
        let summary = svn_analyze::walk_template(&fragment, &source);
        let emitted = svn_emit::emit_document(&doc, &fragment, &summary, file);
        inputs.push(svn_typecheck::CheckInput {
            source_path: file.clone(),
            generated_ts: emitted.typescript,
            line_map: emitted.line_map,
        });
    }

    let mut diagnostics = match svn_typecheck::check(workspace, tsconfig, &inputs) {
        Ok(d) => d,
        Err(err) => {
            eprintln!("svelte-check-native: type-check failed: {err}");
            return ExitCode::from(2);
        }
    };

    // `--threshold error` drops warnings entirely (mirrors upstream).
    if threshold == "error" {
        diagnostics.retain(|d| matches!(d.severity, svn_typecheck::Severity::Error));
    }

    let error_count = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Error))
        .count();
    let warning_count = diagnostics.len() - error_count;

    print_diagnostics(workspace, &diagnostics, output_format);

    if error_count > 0 || (fail_on_warnings && warning_count > 0) {
        ExitCode::from(1)
    } else {
        ExitCode::from(0)
    }
}

fn print_diagnostics(
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    output_format: &str,
) {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let errors = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Error))
        .count();
    let warnings = diagnostics.len() - errors;
    let files_with_problems: std::collections::HashSet<_> =
        diagnostics.iter().map(|d| &d.source_path).collect();
    let use_color = std::io::IsTerminal::is_terminal(&std::io::stdout());

    match output_format {
        "machine-verbose" => {
            print_machine(workspace, diagnostics, now_ms, true);
            print_machine_completed(
                now_ms,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "machine" => {
            print_machine(workspace, diagnostics, now_ms, false);
            print_machine_completed(
                now_ms,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "human" => {
            print_human(workspace, diagnostics, false, use_color);
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
            print_human(workspace, diagnostics, true, use_color);
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
) {
    println!("{now_ms} START \"{}\"", workspace.display());
    for d in diagnostics {
        let rel = d
            .source_path
            .strip_prefix(workspace)
            .unwrap_or(&d.source_path);
        let type_label = match d.severity {
            svn_typecheck::Severity::Error => "ERROR",
            svn_typecheck::Severity::Warning => "WARNING",
        };
        if verbose {
            let payload = serde_json::json!({
                "type": type_label,
                "filename": rel.to_string_lossy(),
                "start": {
                    "line": d.line.saturating_sub(1),
                    "character": d.column.saturating_sub(1),
                },
                "end": {
                    "line": d.line.saturating_sub(1),
                    "character": d.column.saturating_sub(1) + d.span_length.unwrap_or(0),
                },
                "message": d.message,
                "code": d.code,
                "source": "ts",
            });
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

fn print_machine_completed(now_ms: u128, errors: usize, warnings: usize, files: usize) {
    println!(
        "{now_ms} COMPLETED 0 FILES {errors} ERRORS {warnings} WARNINGS {files} FILES_WITH_PROBLEMS"
    );
}

/// `human` / `human-verbose` body — per-diagnostic block.
fn print_human(
    workspace: &Path,
    diagnostics: &[svn_typecheck::CheckDiagnostic],
    verbose: bool,
    color: bool,
) {
    let workspace_display = workspace.display().to_string();
    for d in diagnostics {
        let rel = d
            .source_path
            .strip_prefix(workspace)
            .unwrap_or(&d.source_path);
        let filename = rel.display().to_string();
        // Path that IDEs turn into clickable links.
        println!(
            "{workspace_display}/{}:{}:{}",
            paint(&filename, "32", color),
            d.line,
            d.column,
        );
        let label = match d.severity {
            svn_typecheck::Severity::Error => paint("Error", "31", color),
            svn_typecheck::Severity::Warning => paint("Warn", "33", color),
        };
        if verbose {
            // Code frame: try to read the source file and emit a short
            // excerpt around the diagnostic line, with a caret pointer.
            let frame = format_code_frame(&d.source_path, d.line, d.column, d.span_length);
            if frame.is_empty() {
                println!("{label}: {} (TS{})", d.message, d.code);
            } else {
                println!(
                    "{label}: {} (TS{})\n{}",
                    d.message,
                    d.code,
                    paint(&frame, "36", color),
                );
            }
        } else {
            println!("{label}: {} (TS{})", d.message, d.code);
        }
        println!();
    }
}

fn print_human_summary(errors: usize, warnings: usize, files: usize, color: bool) {
    let parts = format!(
        "svelte-check found {} error{} and {} warning{} in {} file{}",
        errors,
        if errors == 1 { "" } else { "s" },
        warnings,
        if warnings == 1 { "" } else { "s" },
        files,
        if files == 1 { "" } else { "s" },
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
fn format_code_frame(
    path: &Path,
    line: u32,
    column: u32,
    span_length: Option<u32>,
) -> String {
    let Ok(source) = std::fs::read_to_string(path) else {
        return String::new();
    };
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
        let _ = std::fmt::Write::write_fmt(
            &mut out,
            format_args!("{ln:>width$} | {content}\n"),
        );
        if ln == line as usize {
            let pad = width + 3 + column.saturating_sub(1) as usize;
            let underline = "^".repeat(span_length.unwrap_or(1).max(1) as usize);
            for _ in 0..pad {
                out.push(' ');
            }
            out.push_str(&underline);
            out.push('\n');
        }
    }
    out
}

/// `--emit-ts` flow: discover `.svelte` files, parse, emit, print to stdout
/// with file separators. Exits 0 unconditionally — debug-mode is best-effort.
fn run_emit_ts(workspace: &Path) -> ExitCode {
    let files = discover_svelte_files(workspace);
    if files.is_empty() {
        eprintln!(
            "svelte-check-native: no .svelte files found under {}",
            workspace.display()
        );
        return ExitCode::from(0);
    }

    for file in &files {
        let source = match std::fs::read_to_string(file) {
            Ok(s) => s,
            Err(err) => {
                eprintln!("failed to read {}: {err}", file.display());
                continue;
            }
        };

        let (doc, parse_errors) = svn_parser::parse_sections(&source);
        for err in &parse_errors {
            eprintln!("{}: parse warning: {err}", file.display());
        }

        let (fragment, template_errors) =
            svn_parser::parse_all_template_runs(&source, &doc.template.text_runs);
        for err in &template_errors {
            eprintln!("{}: template warning: {err}", file.display());
        }

        let summary = svn_analyze::walk_template(&fragment, &source);
        let emitted = svn_emit::emit_document(&doc, &fragment, &summary, file);
        let display_path = file
            .strip_prefix(workspace)
            .unwrap_or(file)
            .display()
            .to_string();
        println!("// === {display_path} ===");
        println!("{}", emitted.typescript);
    }

    ExitCode::from(0)
}

fn discover_svelte_files(workspace: &Path) -> Vec<PathBuf> {
    WalkDir::new(workspace)
        .into_iter()
        .filter_entry(|e| !is_excluded_dir(e.path()))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let path = e.path();
            matches!(path.extension().and_then(|s| s.to_str()), Some("svelte"))
        })
        .map(|e| e.path().to_path_buf())
        .collect()
}

fn is_excluded_dir(path: &Path) -> bool {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    matches!(
        name,
        "node_modules" | ".git" | ".svelte-kit" | ".svelte-check" | "target" | "dist"
    ) || name.starts_with('.')
}
