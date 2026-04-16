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

    /// Print phase-by-phase timing breakdown (discovery, parse+emit,
    /// tsgo, compiler bridge) at the end of the run.
    #[arg(long, default_value_t = false)]
    timings: bool,

    /// Print resolved paths (workspace, tsconfig, tsgo, JS runtime,
    /// svelte/compiler) and exit. Useful for diagnosing "which tsgo
    /// did it pick?" issues.
    #[arg(long = "debug-paths", default_value_t = false)]
    debug_paths: bool,

    /// Print the resolved tsgo binary path + its --version output, then
    /// exit. Helps verify that `@typescript/native-preview` is at the
    /// expected version.
    #[arg(long = "tsgo-version", default_value_t = false)]
    tsgo_version: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Coding-agent CLIs set marker env vars on spawned subprocesses so child
    // tools can adapt their output. Upstream svelte-check honors CLAUDECODE=1;
    // we extend the same machine-output default to Gemini CLI (GEMINI_CLI=1)
    // and OpenAI Codex CLI (CODEX_CI=1) since they consume tool output the
    // same way.
    let in_agent_cli = ["CLAUDECODE", "GEMINI_CLI", "CODEX_CI"]
        .iter()
        .any(|k| std::env::var(k).as_deref() == Ok("1"));
    let output = if in_agent_cli {
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

    if cli.tsgo_version {
        return run_tsgo_version(&workspace);
    }

    let tsconfig = match resolve_tsconfig(&workspace, cli.tsconfig.as_deref(), cli.no_tsconfig) {
        Ok(Some(p)) => Some(p),
        Ok(None) => None,
        Err(msg) => {
            eprintln!("svelte-check-native: {msg}");
            return ExitCode::from(2);
        }
    };

    if cli.debug_paths {
        return run_debug_paths(&workspace, tsconfig.as_deref());
    }

    let Some(tsconfig) = tsconfig else {
        eprintln!(
            "svelte-check-native: --no-tsconfig mode is not yet implemented; pass --tsconfig <path> or run inside a project with a tsconfig.json"
        );
        return ExitCode::from(2);
    };

    let diagnostic_sources = match parse_diagnostic_sources(cli.diagnostic_sources.as_deref()) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("svelte-check-native: {msg}");
            return ExitCode::from(2);
        }
    };
    let compiler_warnings = parse_compiler_warnings(cli.compiler_warnings.as_deref());
    let ignore_set = build_ignore_set(cli.ignore.as_deref());
    let color = resolve_color_mode(cli.color, cli.no_color);

    run_typecheck(
        &workspace,
        &tsconfig,
        &output,
        &cli.threshold,
        cli.fail_on_warnings,
        diagnostic_sources,
        &compiler_warnings,
        ignore_set.as_ref(),
        color,
        cli.timings,
    )
}

/// Tri-state color mode resolved from `--color` / `--no-color` / isatty.
///
/// `--no-color` wins (most defensive — explicit opt-out always honored).
/// `--color` forces ANSI even when stdout is piped (useful for CI tools
/// that render ANSI in their UI). Otherwise auto-detect via isatty.
#[derive(Debug, Clone, Copy)]
enum ColorMode {
    Always,
    Never,
    Auto,
}

impl ColorMode {
    fn use_color(self) -> bool {
        match self {
            Self::Always => true,
            Self::Never => false,
            Self::Auto => std::io::IsTerminal::is_terminal(&std::io::stdout()),
        }
    }
}

fn resolve_color_mode(force_on: bool, force_off: bool) -> ColorMode {
    if force_off {
        ColorMode::Never
    } else if force_on {
        ColorMode::Always
    } else {
        ColorMode::Auto
    }
}

/// `--debug-paths`: print every resolved binary / file the run would
/// use, then exit. Useful when "which tsgo did it pick?" or "is bun
/// even being found?" comes up.
fn run_debug_paths(workspace: &Path, tsconfig: Option<&Path>) -> ExitCode {
    println!("workspace:        {}", workspace.display());
    match tsconfig {
        Some(p) => println!("tsconfig:         {}", p.display()),
        None => println!("tsconfig:         <none>"),
    }
    match svn_typecheck::discover(workspace) {
        Ok(bin) => println!("tsgo:             {}", &bin.path.display()),
        Err(e) => println!("tsgo:             <not found> ({e})"),
    }
    // The svelte-compiler crate keeps its discovery internal; report
    // best-effort by checking the same env var + PATH lookups it does.
    let runtime = std::env::var("SVN_JS_RUNTIME").ok();
    if let Some(r) = &runtime {
        println!("js runtime:       {r} (from SVN_JS_RUNTIME)");
    } else if let Ok(p) = which_on_path("bun") {
        println!("js runtime:       {} (bun)", p.display());
    } else if let Ok(p) = which_on_path("node") {
        println!("js runtime:       {} (node)", p.display());
    } else {
        println!("js runtime:       <not found> (compiler warnings will be skipped)");
    }
    ExitCode::from(0)
}

/// `--tsgo-version`: print resolved binary path + `tsgo --version`,
/// exit. No type-checking happens.
fn run_tsgo_version(workspace: &Path) -> ExitCode {
    let bin = match svn_typecheck::discover(workspace) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("svelte-check-native: tsgo not found: {e}");
            return ExitCode::from(2);
        }
    };
    println!("tsgo binary: {}", &bin.path.display());
    let output = std::process::Command::new(&bin.path)
        .arg("--version")
        .output();
    match output {
        Ok(o) => {
            let stdout = String::from_utf8_lossy(&o.stdout);
            let stderr = String::from_utf8_lossy(&o.stderr);
            if !stdout.trim().is_empty() {
                println!("tsgo version: {}", stdout.trim());
            }
            if !stderr.trim().is_empty() {
                eprintln!("{}", stderr.trim());
            }
            if o.status.success() {
                ExitCode::from(0)
            } else {
                ExitCode::from(1)
            }
        }
        Err(e) => {
            eprintln!("svelte-check-native: failed to invoke tsgo: {e}");
            ExitCode::from(2)
        }
    }
}

/// Tiny `which` reimplementation to avoid pulling in a dep solely for
/// `--debug-paths`. Walks `PATH` looking for an executable.
fn which_on_path(name: &str) -> std::io::Result<PathBuf> {
    let path = std::env::var_os("PATH")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "PATH not set"))?;
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        name.to_string(),
    ))
}

/// Which diagnostic sources are active. Defaults to all-enabled.
///
/// `--diagnostic-sources "js,svelte"` switches off `css` (and disables
/// any source not named in the list). `js` covers TS too — they share
/// the same backend for us.
#[derive(Debug, Clone, Copy)]
struct DiagnosticSources {
    js: bool,
    svelte: bool,
    css: bool,
}

// `DiagnosticSources::all()` previously existed as a default
// constructor — now subsumed by parse_diagnostic_sources(None) which
// only enables the sources we actually support (js + svelte; css is
// reserved but not yet implemented).

/// Parse `--diagnostic-sources "js,svelte"` into our enabled-source set.
///
/// Returns `Err` with a user-facing message when an unsupported source
/// is requested (currently `css` — we don't ship a CSS linter yet).
/// Empty entries are skipped silently. Unknown entries warn-and-continue.
///
/// When `spec` is `None`, all currently-supported sources are enabled.
fn parse_diagnostic_sources(spec: Option<&str>) -> Result<DiagnosticSources, String> {
    let Some(spec) = spec else {
        // Default = everything we actually support. `css` is reserved
        // and stays off so we don't claim to lint CSS when we don't.
        return Ok(DiagnosticSources {
            js: true,
            svelte: true,
            css: false,
        });
    };
    let mut sources = DiagnosticSources {
        js: false,
        svelte: false,
        css: false,
    };
    for entry in spec.split(',') {
        let entry = entry.trim().to_lowercase();
        match entry.as_str() {
            "js" | "ts" | "javascript" | "typescript" => sources.js = true,
            "svelte" => sources.svelte = true,
            "css" | "scss" | "sass" | "less" | "postcss" => {
                // Hard error rather than silent no-op: if the user
                // explicitly asks for css linting, telling them we'll
                // do something and then doing nothing is worse than
                // making them notice the gap.
                return Err(format!(
                    "--diagnostic-sources {entry:?} requested but CSS linting is not yet \
                     implemented. Drop {entry:?} from the list (or omit --diagnostic-sources \
                     entirely to use the supported defaults: js, svelte)."
                ));
            }
            "" => {}
            other => {
                eprintln!(
                    "svelte-check-native: unknown --diagnostic-sources entry {other:?}; ignoring"
                );
            }
        }
    }
    Ok(sources)
}

/// Per-compiler-warning severity override map.
///
/// `--compiler-warnings "css-unused-selector:ignore,unused-export-let:error"`
/// → `{ "css-unused-selector": Ignore, "unused-export-let": Error }`.
/// Anything not listed keeps its compiler-default severity (`warning`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CompilerWarningOverride {
    Ignore,
    Warning,
    Error,
}

/// Apply a per-code override to a single compiler-warning severity.
/// Returns `None` if the override is `Ignore` (caller should drop the
/// diagnostic).
fn apply_compiler_override(
    code: &str,
    base: svn_svelte_compiler::Severity,
    overrides: &std::collections::HashMap<String, CompilerWarningOverride>,
) -> Option<svn_typecheck::Severity> {
    overrides
        .get(code)
        .copied()
        .map(|o| match o {
            CompilerWarningOverride::Ignore => None,
            CompilerWarningOverride::Warning => Some(svn_typecheck::Severity::Warning),
            CompilerWarningOverride::Error => Some(svn_typecheck::Severity::Error),
        })
        .unwrap_or_else(|| {
            Some(match base {
                svn_svelte_compiler::Severity::Error => svn_typecheck::Severity::Error,
                svn_svelte_compiler::Severity::Warning => svn_typecheck::Severity::Warning,
            })
        })
}

fn parse_compiler_warnings(
    spec: Option<&str>,
) -> std::collections::HashMap<String, CompilerWarningOverride> {
    let mut out = std::collections::HashMap::new();
    let Some(spec) = spec else {
        return out;
    };
    for entry in spec.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        let Some((code, severity)) = entry.split_once(':') else {
            eprintln!(
                "svelte-check-native: malformed --compiler-warnings entry {entry:?} (expected `code:severity`); ignoring"
            );
            continue;
        };
        let severity = match severity.trim().to_lowercase().as_str() {
            "ignore" | "off" | "silent" => CompilerWarningOverride::Ignore,
            "warning" | "warn" => CompilerWarningOverride::Warning,
            "error" => CompilerWarningOverride::Error,
            other => {
                eprintln!(
                    "svelte-check-native: unknown --compiler-warnings severity {other:?}; ignoring entry"
                );
                continue;
            }
        };
        out.insert(code.trim().to_string(), severity);
    }
    out
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
/// svelte-check). `sources` opts diagnostics in/out per source family
/// (`js`/`svelte`/`css`). `compiler_overrides` reclassifies individual
/// compiler warnings (e.g. `css-unused-selector:error`). `ignore` is a
/// pre-built glob set for path filtering; `color` controls ANSI in
/// human output; `timings` prints a phase-by-phase breakdown when
/// true.
#[allow(clippy::too_many_arguments)]
fn run_typecheck(
    workspace: &Path,
    tsconfig: &Path,
    output_format: &str,
    threshold: &str,
    fail_on_warnings: bool,
    sources: DiagnosticSources,
    compiler_overrides: &std::collections::HashMap<String, CompilerWarningOverride>,
    ignore: Option<&globset::GlobSet>,
    color: ColorMode,
    timings: bool,
) -> ExitCode {
    let phase_start = std::time::Instant::now();

    let mark = std::time::Instant::now();
    let svelte_files = discover_svelte_files(workspace, ignore);
    let t_discovery = mark.elapsed();

    // Read every source up-front; we need the bytes for both the
    // tsgo-typecheck path and the svelte/compiler bridge.
    let mut svelte_sources: Vec<(PathBuf, String)> = Vec::with_capacity(svelte_files.len());
    for file in &svelte_files {
        match std::fs::read_to_string(file) {
            Ok(s) => svelte_sources.push((file.clone(), s)),
            Err(err) => {
                eprintln!("failed to read {}: {err}", file.display());
            }
        }
    }

    let mark = std::time::Instant::now();
    let mut inputs: Vec<svn_typecheck::CheckInput> = Vec::with_capacity(svelte_sources.len());
    for (file, source) in &svelte_sources {
        let (doc, _parse_errors) = svn_parser::parse_sections(source);
        let (fragment, _template_errors) =
            svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
        let summary = svn_analyze::walk_template(&fragment, source);
        let emitted = svn_emit::emit_document(&doc, &fragment, &summary, file);
        inputs.push(svn_typecheck::CheckInput {
            source_path: file.clone(),
            generated_ts: emitted.typescript,
            line_map: emitted.line_map,
        });
    }
    let t_emit = mark.elapsed();

    // Run tsgo (`js`/`ts` source). Skipped entirely when
    // `--diagnostic-sources` opts out of `js`.
    let mark = std::time::Instant::now();
    let mut diagnostics = if sources.js {
        match svn_typecheck::check(workspace, tsconfig, &inputs) {
            Ok(d) => d,
            Err(err) => {
                eprintln!("svelte-check-native: type-check failed: {err}");
                return ExitCode::from(2);
            }
        }
    } else {
        Vec::new()
    };
    let t_typecheck = mark.elapsed();

    // Compiler-warning bridge: ask the user's `svelte/compiler` for any
    // non-typecheck diagnostics (`state_referenced_locally`,
    // `element_invalid_self_closing_tag`, accessibility hints, etc.).
    // Skipped when `--diagnostic-sources` opts out of `svelte`. Each
    // emitted warning gets routed through `apply_compiler_override`
    // first so `--compiler-warnings code:severity` reclassifications
    // win.
    let mark = std::time::Instant::now();
    if sources.svelte {
        match svn_svelte_compiler::compile_batch(workspace, &svelte_sources) {
            Ok(per_file) => {
                for (path, warnings) in per_file {
                    for w in warnings {
                        let severity =
                            apply_compiler_override(&w.code, w.severity, compiler_overrides);
                        let Some(severity) = severity else { continue };
                        // Documented compiler-warning codes link to the
                        // svelte.dev compiler-warnings reference page
                        // via their slug. Mirrors what upstream svelte-
                        // check emits in `codeDescription.href`.
                        let href = format!(
                            "https://svelte.dev/docs/svelte/compiler-warnings#{}",
                            w.code,
                        );
                        // svelte/compiler emits 1-based line numbers
                        // but 0-based column offsets. CheckDiagnostic
                        // is documented as 1-based across the board
                        // (and the formatter subtracts 1 to convert
                        // back to 0-based LSP-style on the way out),
                        // so add 1 to columns at the source-of-truth
                        // boundary.
                        diagnostics.push(svn_typecheck::CheckDiagnostic {
                            source_path: path.clone(),
                            line: w.start.line,
                            column: w.start.column.saturating_add(1),
                            end_line: w.end.line,
                            end_column: w.end.column.saturating_add(1),
                            severity,
                            code: svn_typecheck::DiagnosticCode::Slug(w.code.clone()),
                            // Raw message — no slug pollution. The slug
                            // surfaces via `code` separately.
                            message: w.message,
                            source: svn_typecheck::DiagnosticSource::Svelte,
                            code_description_url: Some(href),
                        });
                    }
                }
            }
            Err(_) => {
                // Bridge unavailable — proceed with TS diagnostics only.
            }
        }
    }
    let t_compiler = mark.elapsed();

    // `css` source is reserved — once we add a CSS linter (or wire one
    // through preprocessor output), this is where we'd run it. For now
    // the flag's effect on `css` is purely opt-out semantics; opting in
    // is a no-op until we have something to emit.
    let _ = sources.css;

    // `--threshold error` drops warnings entirely (mirrors upstream).
    if threshold == "error" {
        diagnostics.retain(|d| matches!(d.severity, svn_typecheck::Severity::Error));
    }

    let error_count = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Error))
        .count();
    let warning_count = diagnostics.len() - error_count;

    print_diagnostics(
        workspace,
        &diagnostics,
        output_format,
        color,
        svelte_files.len(),
    );

    if timings {
        let total = phase_start.elapsed();
        eprintln!();
        eprintln!("Phase                        Duration");
        eprintln!("─────────────────────────────────────");
        eprintln!("discovery                    {:>9.2?}", t_discovery);
        eprintln!("parse + analyze + emit       {:>9.2?}", t_emit);
        eprintln!("tsgo type-check              {:>9.2?}", t_typecheck);
        eprintln!("svelte/compiler bridge       {:>9.2?}", t_compiler);
        eprintln!("─────────────────────────────────────");
        eprintln!("total (incl. format/exit)    {:>9.2?}", total);
        eprintln!(
            "files: {} | errors: {} | warnings: {}",
            svelte_files.len(),
            error_count,
            warning_count,
        );
    }

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
    color: ColorMode,
    files_checked: usize,
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
    let use_color = color.use_color();

    match output_format {
        "machine-verbose" => {
            print_machine(workspace, diagnostics, now_ms, true);
            print_machine_completed(
                now_ms,
                files_checked,
                errors,
                warnings,
                files_with_problems.len(),
            );
        }
        "machine" => {
            print_machine(workspace, diagnostics, now_ms, false);
            print_machine_completed(
                now_ms,
                files_checked,
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
        // Span length for the code-frame caret. We have a real
        // [start, end) so prefer that; fall back to 1 char when the
        // span is empty (zero-width markers still get visualized).
        let span = d.end_column.saturating_sub(d.column);
        let span = if span == 0 { Some(1) } else { Some(span) };
        if verbose {
            // Code frame: try to read the source file and emit a short
            // excerpt around the diagnostic line, with a caret pointer.
            let frame = format_code_frame(&d.source_path, d.line, d.column, span);
            // `Display for DiagnosticCode` already prefixes numeric
            // codes with `TS`; printing `(TS{code})` would double-up
            // for TS errors AND attach a wrong `TS` to slug codes.
            // Just print `(<code>)` and let Display do the right thing.
            if frame.is_empty() {
                println!("{label}: {} ({})", d.message, d.code);
            } else {
                println!(
                    "{label}: {} ({})\n{}",
                    d.message,
                    d.code,
                    paint(&frame, "36", color),
                );
            }
        } else {
            println!("{label}: {} ({})", d.message, d.code);
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
fn format_code_frame(path: &Path, line: u32, column: u32, span_length: Option<u32>) -> String {
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
        let _ = std::fmt::Write::write_fmt(&mut out, format_args!("{ln:>width$} | {content}\n"));
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
    let files = discover_svelte_files(workspace, None);
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

fn discover_svelte_files(workspace: &Path, ignore: Option<&globset::GlobSet>) -> Vec<PathBuf> {
    WalkDir::new(workspace)
        .into_iter()
        .filter_entry(|e| {
            // Hard-coded exclusions for directories that are NEVER worth
            // walking (node_modules, .git, build outputs).
            if is_excluded_dir(e.path()) {
                return false;
            }
            // User --ignore patterns. Match against the workspace-relative
            // path so patterns like "dist" / "**/*.spec.svelte" /
            // "_components/legacy/**" all behave intuitively.
            if let Some(set) = ignore {
                if let Ok(rel) = e.path().strip_prefix(workspace) {
                    if set.is_match(rel) {
                        return false;
                    }
                }
            }
            true
        })
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
        .filter(|e| {
            let path = e.path();
            matches!(path.extension().and_then(|s| s.to_str()), Some("svelte"))
        })
        // Per-file glob check too, so `*.spec.svelte`-style patterns
        // exclude individual files (not just directories).
        .filter(|e| {
            let Some(set) = ignore else { return true };
            match e.path().strip_prefix(workspace) {
                Ok(rel) => !set.is_match(rel),
                Err(_) => true,
            }
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

/// Build a [`GlobSet`] from a comma-separated `--ignore` spec.
///
/// Patterns are git-style globs (`**/*` for arbitrary depth, `*` for
/// single segment, `?` for one char, `[abc]` for character classes).
/// Matched against workspace-relative paths.
///
/// Empty / whitespace-only patterns are skipped. Invalid patterns
/// produce a stderr warning and are silently dropped — the run
/// continues with the patterns that DID parse, mirroring upstream
/// svelte-check's lenient behavior.
fn build_ignore_set(spec: Option<&str>) -> Option<globset::GlobSet> {
    let spec = spec?;
    let mut builder = globset::GlobSetBuilder::new();
    let mut any = false;
    for entry in spec.split(',') {
        let entry = entry.trim();
        if entry.is_empty() {
            continue;
        }
        match globset::Glob::new(entry) {
            Ok(g) => {
                builder.add(g);
                any = true;
            }
            Err(e) => {
                eprintln!("svelte-check-native: invalid --ignore pattern {entry:?}: {e}");
            }
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}
