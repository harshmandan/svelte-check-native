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
#[command(name = "svelte-check-native", version)]
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
    let _output = if std::env::var("CLAUDECODE").as_deref() == Ok("1") {
        "machine".to_string()
    } else {
        cli.output.clone()
    };

    let workspace = cli
        .workspace
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));

    if cli.emit_ts {
        return run_emit_ts(&workspace);
    }

    // Type-checking pipeline still under construction.
    eprintln!("svelte-check-native: type-checking not yet implemented (phase 1 in progress)");
    eprintln!("see todo.md or https://github.com/harshmandan/svelte-check-native");
    ExitCode::from(2)
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

        let emitted = svn_emit::emit_document(&doc);
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
