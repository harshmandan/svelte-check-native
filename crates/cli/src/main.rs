//! `svelte-check-native` — CLI entrypoint.
//!
//! Phase 1: minimum useful command surface.
//!
//! Currently supports `--emit-ts` (debug print of generated TypeScript) and
//! accepts the broader flag set required by the bug-fixtures and
//! upstream-sanity test runners. Type-checking diagnostics (the actual
//! purpose of the binary) land once the emit + analyze + typecheck crates
//! are wired together.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

mod collisions;
mod discovery;
mod output;
mod svelte_config;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;
use rayon::prelude::*;

use collisions::rewrite_svelte_imports_for_collisions;
use discovery::{discover_relevant_files, discover_svelte_files, path_is_under_node_modules};
use output::{print_diagnostics, print_machine_failure};

#[derive(Parser, Debug)]
#[command(
    name = "svelte-check-native",
    version,
    about = "CLI-only type checker for Svelte 5+ projects. Powered by tsgo.",
    long_about = "svelte-check-native — type-check Svelte components.\n\n\
                  Both Svelte 4 (export let, $:, <slot>, on:event) and Svelte 5\n\
                  (runes) syntax are supported. tsgo (@typescript/native-preview)\n\
                  must be installed in the project's node_modules, or pointed at\n\
                  via TSGO_BIN. A tsconfig/jsconfig is required (mirrors\n\
                  `svelte-check --tsgo`); --no-tsconfig and watch mode are not\n\
                  supported."
)]
struct Cli {
    /// Workspace root to scan. Defaults to current working directory.
    /// Accepts `--projectRoot` and `--workspace-root` as upstream-compat
    /// aliases.
    #[arg(long, visible_aliases = ["projectRoot", "workspace-root"])]
    workspace: Option<PathBuf>,

    /// Path to tsconfig.json (or jsconfig.json). When omitted, walks up from
    /// `--workspace` looking for one.
    #[arg(long)]
    tsconfig: Option<PathBuf>,

    /// Path to a `svelte.config` / `vite.config` file, for projects whose
    /// config has a non-standard name or location. Relative paths resolve
    /// against `--workspace`. When given, it overrides the workspace-dir
    /// discovery (and disables nested config lookup), mirroring upstream
    /// svelte-check's `--config` (added in #3031 / #3066). For a
    /// `vite.config`, the Svelte-config surface we consume
    /// (`warningFilter`, `kit.files`, `namespace`) is read from the inline
    /// `sveltekit()` / `svelte()` plugin options when statically
    /// resolvable (SvelteKit 2.62+, svelte/kit#15944); otherwise it falls
    /// back to the `svelte.config`-style default export.
    #[arg(long)]
    config: Option<PathBuf>,

    /// Upstream's tsconfig-less mode. Recognised so it isn't a generic
    /// parse error, but NOT supported: like `svelte-check --tsgo`
    /// (which we mirror), a tsconfig/jsconfig is required — upstream's
    /// own tsgo path rejects `--no-tsconfig` (index.ts:441). Passing it
    /// exits 2 with a clear message rather than a clap error.
    #[arg(long = "no-tsconfig", default_value_t = false)]
    no_tsconfig: bool,

    /// Files/folders to ignore (comma-separated, relative to workspace).
    /// Upstream only honors this with `--no-tsconfig`; since we don't
    /// support that mode, `--ignore` is recognised but rejected with
    /// upstream's coupling message rather than a generic clap error.
    #[arg(long)]
    ignore: Option<String>,

    /// Watch mode. Out of scope (use an external watcher like
    /// watchexec). Recognised so it's a clear rejection, not a clap
    /// parse error.
    #[arg(long, default_value_t = false)]
    watch: bool,

    /// Watch-mode screen-clear toggle. Out of scope along with
    /// `--watch`; recognised only to give a clear rejection.
    #[arg(long = "preserveWatchOutput", default_value_t = false)]
    preserve_watch_output: bool,

    /// Output format. Accepted values match upstream svelte-check.
    /// Default is `human-verbose` interactively, or `machine` when
    /// invoked from an agent CLI (CLAUDECODE/GEMINI_CLI/CODEX_CI=1).
    /// An explicit `--output` always wins.
    #[arg(long)]
    output: Option<String>,

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

    /// Exit non-zero on errors. Accepted for upstream-compat — errors
    /// always cause a non-zero exit regardless of this flag, so it's
    /// effectively a no-op in our binary. Still recognised so CI
    /// scripts ported from upstream don't reject the flag.
    #[arg(long = "fail-on-errors", default_value_t = false)]
    fail_on_errors: bool,

    /// Exit non-zero on hint-severity diagnostics. Accepted for
    /// upstream-compat. We don't currently emit hint-severity
    /// diagnostics — Svelte 5's compiler doesn't either; the
    /// severity exists in upstream's surface for future use. The
    /// flag is recognised so CI scripts ported from upstream don't
    /// reject it; it has no behavioural effect today.
    #[arg(long = "fail-on-hints", default_value_t = false)]
    fail_on_hints: bool,

    /// Compiler-warning severity overrides (`code:severity,code:severity`).
    #[arg(long = "compiler-warnings")]
    compiler_warnings: Option<String>,

    /// Drop every Svelte compiler warning whose source path contains
    /// a `node_modules/` segment. Mirrors the common upstream pattern
    /// `compilerOptions.warningFilter: (w) => !w.filename?.includes('node_modules')`.
    /// Our default workspace scan already skips `node_modules/`
    /// directories; this flag is belt-and-suspenders for cases where
    /// symlinks (e.g. pnpm workspaces) put node_modules files in
    /// scope anyway.
    #[arg(long = "ignore-node-modules-warnings", default_value_t = false)]
    ignore_node_modules_warnings: bool,

    /// Enable disk caching. No-op for us — caching is always on; accepted
    /// for upstream-compat.
    #[arg(long, default_value_t = false)]
    incremental: bool,

    /// Use tsgo. No-op for us — tsgo is always on; accepted for
    /// upstream-compat.
    #[arg(long, default_value_t = false)]
    tsgo: bool,

    /// Use tsgo via its experimental in-process API (upstream #3036 /
    /// #3067). No-op for us — our pipeline already drives tsgo directly,
    /// so this produces output identical to the default path. Accepted
    /// (not rejected) because upstream svelte-check's own `test-sanity.js`
    /// — which we reuse byte-for-byte as the `upstream_sanity` parity gate
    /// — exercises this flag and expects the same diagnostics as without
    /// it. Dropping it broke that gate at the 32556dbf pin.
    #[arg(long = "tsgo-experimental-api", default_value_t = false)]
    tsgo_experimental_api: bool,

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

    /// How to source compile warnings. `native` (default) runs our
    /// in-process Rust lint pass — no subprocess, fast. `bridge`
    /// spawns bun/node workers that import the user's `svelte/compiler`
    /// directly; slower (~+1.5-2s cold), but matches upstream
    /// byte-for-byte including `css_unused_selector` and any
    /// just-released codes our native port hasn't covered yet.
    #[arg(long = "svelte-warnings", default_value = "native")]
    svelte_warnings: String,

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

    /// Print tsgo's extended compilation diagnostics (file/line/symbol
    /// counts, memory use, phase timings) after the normal output.
    /// Passes `--extendedDiagnostics` through to the tsgo subprocess
    /// and prints the trailing stats block verbatim. Useful for
    /// performance investigation on large projects.
    #[arg(long = "tsgo-diagnostics", default_value_t = false)]
    tsgo_diagnostics: bool,

    /// Print every `.svelte` + SvelteKit Kit file the enumeration
    /// finds (one absolute path per line), then exit. Used by the
    /// `kit_file_parity` integration test to pin our discovery
    /// against upstream's.
    #[arg(long = "list-relevant", default_value_t = false, hide = true)]
    list_relevant: bool,

    /// Surface hint-severity diagnostics — TS6133 / 6192 / 6196
    /// (declared-but-never-read / unused-imports), TS6385 / 6387
    /// (deprecation hints). Default off, mirroring upstream
    /// svelte-check's CLI writer (which drops severity ≠
    /// ERROR/WARNING per writers.ts:166-171). With this flag,
    /// tsgo gets `--noUnusedLocals` and `--noUnusedParameters`
    /// appended to its argv so the codes fire from the CLI, and
    /// the surfaced diagnostics are tagged `severity: Hint`
    /// (printed as `HINT` in machine-output, severity 4 in
    /// machine-verbose JSON), matching upstream LS's
    /// `getSuggestionDiagnostics` semantics. Required for the
    /// `language-server`-style fixtures that expect hint codes
    /// (`$bindable-reassign.v5`, `deprecated-unused-hints`,
    /// etc.).
    #[arg(long = "include-suggestions", default_value_t = false)]
    include_suggestions: bool,
}

fn main() -> ExitCode {
    let cli = Cli::parse();

    // Reject recognised-but-unsupported upstream flags with a clear
    // message instead of a generic clap parse error. Watch mode is out
    // of scope (CLAUDE.md — use an external watcher). `--no-tsconfig`
    // and `--ignore` are tsconfig-less-mode surface that upstream's own
    // tsgo path rejects (a tsconfig is required), and we mirror
    // `svelte-check --tsgo`.
    if cli.watch || cli.preserve_watch_output {
        eprintln!(
            "svelte-check-native: watch mode is not supported. Run an external \
             watcher (e.g. `watchexec -- svelte-check-native`) instead."
        );
        return ExitCode::from(2);
    }
    if cli.no_tsconfig {
        eprintln!(
            "svelte-check-native: --no-tsconfig is not supported. A tsconfig/\
             jsconfig is required (this mirrors `svelte-check --tsgo`, whose \
             tsgo path also requires one)."
        );
        return ExitCode::from(2);
    }
    if cli.ignore.is_some() {
        // Upstream: `--ignore` only takes effect with `--no-tsconfig`
        // (options.ts:88). Since we don't support that mode, `--ignore`
        // can never apply — surface upstream's exact constraint.
        eprintln!(
            "svelte-check-native: --ignore only has an effect when using \
             --no-tsconfig, which is not supported (a tsconfig is required)."
        );
        return ExitCode::from(2);
    }
    if cli.tsgo_experimental_api {
        // Accepted for drop-in compat (upstream's test-sanity.js passes
        // it), but redundant: there's no alternate engine to select.
        // Printed in warning yellow (33) — same color the tool uses for
        // WARNING diagnostics — honoring --color/--no-color; Auto checks
        // stderr since the notice goes there.
        let color = match resolve_color_mode(cli.color, cli.no_color) {
            ColorMode::Always => true,
            ColorMode::Never => false,
            ColorMode::Auto => std::io::IsTerminal::is_terminal(&std::io::stderr()),
        };
        let msg = "svelte-check-native: --tsgo-experimental-api isn't needed — svelte-check-native always type-checks with tsgo.";
        if color {
            eprintln!("\x1b[33m{msg}\x1b[0m");
        } else {
            eprintln!("{msg}");
        }
    }

    // Coding-agent CLIs set marker env vars on spawned subprocesses so child
    // tools can adapt their output. Upstream svelte-check honors CLAUDECODE=1;
    // we extend the same machine-output default to Gemini CLI (GEMINI_CLI=1)
    // and OpenAI Codex CLI (CODEX_CI=1) since they consume tool output the
    // same way.
    //
    // The override only fires when the user didn't pass `--output`. An
    // explicit `--output machine-verbose` (e.g. from scripts/bench.mjs)
    // must reach the formatter unchanged — pre-fix, the agent-env check
    // silently downgraded verbose JSON to the line-oriented `machine`
    // format, breaking any caller's JSON parser.
    let in_agent_cli = ["CLAUDECODE", "GEMINI_CLI", "CODEX_CI"]
        .iter()
        .any(|k| std::env::var(k).as_deref() == Ok("1"));
    const OUTPUT_FORMATS: [&str; 4] = ["human", "human-verbose", "machine", "machine-verbose"];
    let output = cli
        .output
        .clone()
        .filter(|o| OUTPUT_FORMATS.contains(&o.as_str()))
        .unwrap_or_else(|| {
            if in_agent_cli {
                "machine".to_string()
            } else {
                "human-verbose".to_string()
            }
        });

    let workspace_arg = cli
        .workspace
        .clone()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."));
    // Canonicalize so subsequent walk-up logic (tsgo discovery, tsconfig
    // search) traverses real filesystem ancestors. Without this, a relative
    // workspace like `./test-success` walks `.parent()` → `./` → `""` →
    // None and never reaches actual node_modules locations.
    //
    // Uses `dunce::canonicalize` so on Windows drive paths come back as
    // `D:\…` rather than the verbatim `\\?\D:\…` form. tsgo silently skips
    // a workspace root passed in verbatim form and our lexical include-
    // glob matching (forward slashes in user patterns) doesn't survive
    // the prefix either — "0 files, 0 errors" on Windows traces back here.
    let workspace = match dunce::canonicalize(&workspace_arg) {
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

    if cli.list_relevant {
        let (svelte, kit, _runes, _user_ts) = discover_relevant_files(&workspace);
        for p in svelte.iter().chain(kit.iter()) {
            println!("{}", p.display());
        }
        return ExitCode::from(0);
    }

    let (tsconfig, escaped_solution) = match resolve_tsconfig(&workspace, cli.tsconfig.as_deref()) {
        Ok(pair) => pair,
        Err(msg) => {
            eprintln!("svelte-check-native: {msg}");
            return ExitCode::from(2);
        }
    };
    // If `resolve_tsconfig` escaped a project-references solution to a
    // sub-app's `tsconfig.json`, redirect the workspace to that sub-app
    // too. Without this, tsgo's cwd stays at the monorepo root and
    // `node_modules` resolution for app-local packages
    // (`@org/types`, workspace-scoped deps) fails from the wrong
    // directory. The overlay cache, kit-file discovery, and diagnostic
    // path-relativization all follow workspace.
    //
    // Gated on `escaped_solution`: an explicit `--tsconfig` pointing
    // into a subdirectory must NOT relocate the workspace — that would
    // silently change the discovery root and the `<N> FILES`
    // denominator. Upstream keeps workspace and tsconfig independent.
    let (workspace, solution_root_tsconfig) = match tsconfig.parent() {
        Some(dir) if escaped_solution && dir != workspace && dir.starts_with(&workspace) => {
            eprintln!(
                "svelte-check-native: redirected workspace to {} (parent of {}) — original looked like a TS project-references solution",
                dir.display(),
                tsconfig.display(),
            );
            // Record the ORIGINAL solution root's tsconfig. Overlay
            // builder consults it to flatten sibling-project
            // references into the overlay's include/exclude/paths,
            // so transitive imports across projects remain visible
            // to tsgo (see svn_core::tsconfig::flatten_references).
            let solution_root = workspace.join("tsconfig.json");
            let solution = if solution_root.is_file() {
                Some(solution_root)
            } else {
                None
            };
            (dir.to_path_buf(), solution)
        }
        _ => (workspace, None),
    };

    if cli.debug_paths {
        return run_debug_paths(&workspace, Some(&tsconfig));
    }

    let diagnostic_sources = match parse_diagnostic_sources(cli.diagnostic_sources.as_deref()) {
        Ok(s) => s,
        Err(msg) => {
            eprintln!("svelte-check-native: {msg}");
            return ExitCode::from(2);
        }
    };
    let compiler_warnings = parse_compiler_warnings(cli.compiler_warnings.as_deref());
    let color = resolve_color_mode(cli.color, cli.no_color);

    // Tier 2: static analysis of svelte.config.js. Single parse →
    // single AST → both extractors (warningFilter for compiler-
    // warning filtering, kit.files for discovery overrides). When
    // found and parseable, both feed downstream — warningFilter
    // augments --compiler-warnings at the filter stage; kit.files
    // paths drive the discovery walker's hooks/params recognition.
    // `--config` overrides the workspace-dir discovery of a Svelte config
    // (upstream svelte-check #3031 / #3066). When given, resolve it
    // against the workspace and require it to exist (invocation error,
    // exit 2 — mirrors upstream's `getConfig` throw); otherwise fall
    // back to discovery.
    // Resolve which config supplies the Svelte/Kit settings, preferring
    // a `vite.config.*`'s inline `sveltekit()` / `svelte()` plugin
    // options over `svelte.config.js` (SvelteKit 2.62+, svelte/kit#15944;
    // mirrors upstream `@sveltejs/load-config`, which reads the Vite
    // plugin's `api.options` first and only falls back to
    // `svelte.config.js` when the plugin exposes no inline options).
    let (analysed_config, svelte_config_summary): (
        Option<PathBuf>,
        svelte_config::SvelteConfigSummary,
    ) = if let Some(p) = cli.config.as_deref() {
        // `--config` names an explicit svelte.config OR vite.config file
        // (upstream svelte-check #3031 / #3066). Require it to exist
        // (exit 2, mirroring upstream's `getConfig` throw), then try the
        // vite-plugin form first and fall back to the svelte.config form.
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            workspace.join(p)
        };
        if !resolved.is_file() {
            eprintln!(
                "svelte-check-native: could not find config file at {}",
                resolved.display()
            );
            return ExitCode::from(2);
        }
        let summary = svelte_config::analyse_vite_config(&resolved)
            .unwrap_or_else(|| svelte_config::analyse(&resolved));
        (Some(resolved), summary)
    } else if let Some((path, summary)) = svelte_config::find_vite_config(&workspace)
        .and_then(|p| svelte_config::analyse_vite_config(&p).map(|s| (p, s)))
    {
        (Some(path), summary)
    } else if let Some(path) = svelte_config::find_svelte_config(&workspace) {
        let summary = svelte_config::analyse(&path);
        (Some(path), summary)
    } else {
        (None, svelte_config::SvelteConfigSummary::default())
    };
    if let Some(cfg) = &analysed_config {
        warn_partial_warning_filter(cfg, &svelte_config_summary.warning_filter_plan);
    }
    // Per-file nearest-config resolution for warningFilter + runes
    // (upstream resolves each document's config via upward search from
    // the file's directory — nearest wins, no merging). The
    // workspace-root resolution above is the fallback; an explicit
    // `--config` pins it for every file.
    let mut config_resolver = svelte_config::ConfigResolver::new(
        workspace.clone(),
        svelte_config::ResolvedConfig {
            warning_filter_plan: svelte_config_summary.warning_filter_plan,
            runes: svelte_config_summary.runes,
        },
        cli.config.is_some(),
    );
    let kit_files_settings = svelte_config_summary.kit_files_settings;
    // Set the project-wide preserve-attribute-case flag (svelte config
    // `namespace: 'foreign'`) ONCE before any (parallel) emit reads it.
    svn_emit::set_preserve_attribute_case(svelte_config_summary.preserve_attribute_case);

    let svelte_warnings_mode = match cli.svelte_warnings.as_str() {
        "bridge" => SvelteWarningsMode::Bridge,
        "native" => SvelteWarningsMode::Native,
        other => {
            eprintln!(
                "svelte-check-native: unknown --svelte-warnings value `{other}` (expected native or bridge)"
            );
            return ExitCode::from(2);
        }
    };

    // Validate the threshold value. Upstream warns and falls back to
    // "warning" rather than hard-failing (options.ts::getThreshold).
    let threshold = match cli.threshold.as_str() {
        "error" | "warning" => cli.threshold.as_str(),
        other => {
            eprintln!(
                "svelte-check-native: invalid threshold \"{other}\", using \"warning\" instead"
            );
            "warning"
        }
    };

    run_typecheck(
        &workspace,
        solution_root_tsconfig.as_deref(),
        &tsconfig,
        &output,
        threshold,
        cli.fail_on_warnings,
        diagnostic_sources,
        &compiler_warnings,
        color,
        cli.timings,
        cli.tsgo_diagnostics,
        svelte_warnings_mode,
        cli.ignore_node_modules_warnings,
        &mut config_resolver,
        &kit_files_settings,
        cli.include_suggestions,
    )
}

/// Stderr notice for a `warningFilter` that only partially translated —
/// shared by the workspace-root config and any nested configs the
/// per-file resolver discovers.
fn warn_partial_warning_filter(config_path: &Path, plan: &svelte_config::WarningFilterPlan) {
    if !plan.partial {
        return;
    }
    eprintln!(
        "svelte-check-native: partial `warningFilter` in {} — one or more branches couldn't be translated. Unrecognised: `{}`. Add `--compiler-warnings code:ignore,…` to cover the rest.",
        config_path.display(),
        plan.unrecognised_excerpt.as_deref().unwrap_or("?")
    );
}

/// How to source compile warnings. Drives the bridge-vs-native
/// dispatch inside `run_typecheck`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SvelteWarningsMode {
    /// Fallback: spawn the multi-worker Node bridge against the
    /// user's `svelte/compiler`. Slower than native but matches
    /// upstream byte-for-byte (including `css_unused_selector` and
    /// any just-released codes our native port hasn't covered yet).
    Bridge,
    /// Default: run the native Rust lint pass in-process.
    Native,
}

/// Tri-state color mode resolved from `--color` / `--no-color` / isatty.
///
/// `--no-color` wins (most defensive — explicit opt-out always honored).
/// `--color` forces ANSI even when stdout is piped (useful for CI tools
/// that render ANSI in their UI). Otherwise auto-detect via isatty.
#[derive(Debug, Clone, Copy)]
pub(crate) enum ColorMode {
    Always,
    Never,
    Auto,
}

impl ColorMode {
    pub(crate) fn use_color(self) -> bool {
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
    // The discovery layer flags JS-wrapper installs (`tsgo.js` under
    // node_modules/@typescript/native-preview/bin/) with
    // `needs_node = true`. Those can't be exec'd directly — we have
    // to spawn `node <path>` instead. The main type-check path at
    // runner.rs honors this; missing it here meant
    // `--tsgo-version` failed to launch on JS-wrapper-only installs
    // (rare today since npm pulls a platform-native package, but
    // still real for environments that opt out of platform packages).
    let output = if bin.needs_node {
        std::process::Command::new("node")
            .arg(&bin.path)
            .arg("--version")
            .output()
    } else {
        std::process::Command::new(&bin.path)
            .arg("--version")
            .output()
    };
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
                // Accept silently — CSS linting isn't yet implemented,
                // but rejecting outright blocks monorepo CI scripts
                // that pass `--diagnostic-sources ts,svelte,css`
                // template-fashion (a real-world pattern when porting
                // configs from upstream svelte-check). Match upstream's
                // user-visible behaviour: the flag is accepted; in our
                // case it's a no-op until the css source path lands.
                // Track in notes/ARCH_PARITY.md (Round 2 #1).
                sources.css = true;
            }
            "" => {}
            // Unknown entries are dropped silently, matching upstream's
            // `.filter(s => diagnosticSources.includes(s))` (options.ts).
            other => {
                let _ = other;
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
    Error,
}

/// Apply a per-code override to a single compiler-warning severity.
/// Returns `None` if the override is `Ignore` (caller should drop the
/// diagnostic).
/// Build the `codeDescription.href` URL for a Svelte-compiler
/// diagnostic. Mirrors upstream svelte-check's `getCodeDescription`
/// (`svelte-check/dist/src/index.js:112550`):
///
/// - Warnings route to `compiler-warnings#<code>`.
/// - Errors route to `compiler-errors#<code>`.
/// - Code must start with a lowercase ASCII letter AND contain `_` or
///   `-`. Filters out numeric TS codes and opaque `unknown` slugs
///   that don't correspond to documented anchors.
/// - Hyphens in the code are normalized to underscores before
///   joining with the URL (matches the svelte.dev doc-anchor naming
///   convention).
///
/// Returns `None` when the code doesn't match upstream's filter —
/// emitting a URL for an undocumented code would send users to a
/// 404 anchor, worse than no link.
fn compiler_code_docs_url(code: &str, severity: svn_typecheck::Severity) -> Option<String> {
    let mut chars = code.chars();
    let first = chars.next()?;
    if !first.is_ascii_lowercase() {
        return None;
    }
    if !code.contains('_') && !code.contains('-') {
        return None;
    }
    let base = match severity {
        svn_typecheck::Severity::Error => "https://svelte.dev/docs/svelte/compiler-errors#",
        // Hint-severity diagnostics surface only via tsgo
        // (`--include-suggestions`), never via the Svelte compiler,
        // so they shouldn't reach this svelte-doc URL builder. Treat
        // any non-error severity as warning for URL purposes.
        svn_typecheck::Severity::Warning | svn_typecheck::Severity::Hint => {
            "https://svelte.dev/docs/svelte/compiler-warnings#"
        }
    };
    Some(format!("{base}{}", code.replace('-', "_")))
}

/// Run the native Svelte diagnostics — fatal compile errors AND lint
/// warnings — over every in-scope source, from a SINGLE parse per file.
///
/// Both concerns run after tsgo and previously parsed the corpus
/// independently (two `parse_sections` + two `parse_all_template_runs`
/// + two `PositionMap` builds per file). They share one parse here:
///
/// 1. **Fatal parse/syntax errors** (`ParseError::is_fatal`). The emit
///    and lint passes recover from and discard these, so a syntactically
///    broken file would check clean — or leak baffling TS noise from a
///    garbage overlay. Upstream throws and reports a single
///    `source: "svelte"` error per file
///    (`getDiagnostics.ts::createParserErrorDiagnostic`); we mirror that
///    — first fatal error wins — record the file in `broken` so the
///    caller drops tsgo's noise for it, and skip the checks below (the
///    AST is garbage, as the old lint pass did via the broken filter).
///
/// 2. **Structural analyze-phase errors** on the clean AST
///    (`svn_analyze::check_const_placement`, …) — `svelte/compiler`
///    `2-analyze` visitors like a misplaced `{@const}`. NOT marked broken:
///    svelte2tsx still produces a usable overlay, so tsgo runs and
///    reports independently.
///
/// 3. **Lint warnings** via [`svn_lint::lint_parsed`], reusing this
///    file's parse and position map.
///
/// Diagnostics are merged in the original two-phase emission order (all
/// fatal/structural diagnostics first, then all warnings, both in source
/// order) so output stays byte-identical to the two separate passes.
///
/// Only `native` mode calls this; `bridge` mode gets the real compiler
/// errors (with upstream-accurate codes) straight from `compile_batch`.
fn emit_native_svelte_diagnostics(
    svelte_sources: &[(PathBuf, String)],
    compiler_overrides: &std::collections::HashMap<String, CompilerWarningOverride>,
    diagnostics: &mut Vec<svn_typecheck::CheckDiagnostic>,
    seen: &mut std::collections::HashSet<(String, PathBuf, u32, u32)>,
    workspace: &Path,
    config_resolver: &svelte_config::ConfigResolver,
    broken: &mut std::collections::HashSet<PathBuf>,
) {
    let compat = svn_lint::detect_for_workspace(workspace);

    /// One file's native diagnostics, all derived from its single parse.
    struct PerFile {
        path: PathBuf,
        /// Fatal-parse + structural (const-placement) diagnostics.
        diags: Vec<svn_typecheck::CheckDiagnostic>,
        /// Set when a fatal parse error made the AST unusable.
        broken: bool,
        /// Lint warnings (empty for broken files — they're skipped).
        warnings: Vec<svn_lint::Warning>,
    }

    // Per-file work is pure compute with no shared mutable state — fan
    // out over rayon. `collect` preserves source order for the merge.
    let per_file: Vec<PerFile> = svelte_sources
        .par_iter()
        .map(|(path, source)| {
            let (doc, section_errors) = svn_parser::parse_sections(source);
            let (fragment, template_errors) =
                svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
            let pm = svn_core::PositionMap::new(source);

            // (1) Fatal parse/syntax error → broken; skip every later
            // check (the AST is garbage). First fatal error wins.
            if let Some(err) = section_errors
                .iter()
                .chain(template_errors.iter())
                .find(|e| e.is_fatal())
            {
                let (start, end) = pm.range_positions(err.range());
                return PerFile {
                    path: path.clone(),
                    diags: vec![svn_typecheck::CheckDiagnostic {
                        source_path: path.clone(),
                        // PositionMap is 0-based (line, UTF-16 char);
                        // CheckDiagnostic is 1-based across the board.
                        line: start.line.saturating_add(1),
                        column: start.character.saturating_add(1),
                        end_line: end.line.saturating_add(1),
                        end_column: end.character.saturating_add(1),
                        severity: svn_typecheck::Severity::Error,
                        code: svn_typecheck::DiagnosticCode::Slug(err.code_slug().to_string()),
                        message: err.to_string(),
                        source: svn_typecheck::DiagnosticSource::Svelte,
                        code_description_url: None,
                    }],
                    broken: true,
                    warnings: Vec::new(),
                };
            }

            // (2) Structural analyze-phase errors on the clean AST. The
            // root template fragment is not a legal const-tag host (its
            // grand-parent is the document Root) — start disallowed.
            let mut placement_errs = Vec::new();
            svn_analyze::check_const_placement(&fragment.nodes, false, &mut placement_errs);
            let diags: Vec<svn_typecheck::CheckDiagnostic> = placement_errs
                .into_iter()
                .map(|e| {
                    let (start, end) = pm.range_positions(e.range);
                    svn_typecheck::CheckDiagnostic {
                        source_path: path.clone(),
                        line: start.line.saturating_add(1),
                        column: start.character.saturating_add(1),
                        end_line: end.line.saturating_add(1),
                        end_column: end.character.saturating_add(1),
                        severity: svn_typecheck::Severity::Error,
                        code: svn_typecheck::DiagnosticCode::Slug(
                            "const_tag_invalid_placement".to_string(),
                        ),
                        message: svn_analyze::CONST_TAG_INVALID_PLACEMENT_MSG.to_string(),
                        source: svn_typecheck::DiagnosticSource::Svelte,
                        code_description_url: Some(
                            "https://svelte.dev/e/const_tag_invalid_placement".to_string(),
                        ),
                    }
                })
                .collect();

            // (3) Lint warnings — reuse the parse and the position map
            // (no second parse_sections / line-index scan per file).
            // The nearest config's compilerOptions.runes forces the
            // mode; `None` keeps lint's auto-detection.
            let config_runes = config_resolver.for_path(path).runes;
            let warnings =
                svn_lint::lint_parsed(&doc, &fragment, source, pm, path, config_runes, compat);

            PerFile {
                path: path.clone(),
                diags,
                broken: false,
                warnings,
            }
        })
        .collect();

    // Phase 1: all fatal/structural diagnostics + broken flags, in
    // source order — matches the old compile-error pass's emission slot.
    let mut warning_files: Vec<(PathBuf, Vec<svn_lint::Warning>)> = Vec::new();
    for pf in per_file {
        diagnostics.extend(pf.diags);
        if pf.broken {
            broken.insert(pf.path);
        } else if !pf.warnings.is_empty() {
            warning_files.push((pf.path, pf.warnings));
        }
    }

    // Phase 2: all warnings, in source order — matches the old lint
    // pass's emission slot. Dedups by `(code, path, line, col)` so
    // `--svelte-warnings=both` doesn't double-report against the bridge.
    for (path, warnings) in warning_files {
        for w in warnings {
            let code = w.code.as_str().to_string();
            // Apply user `--compiler-warnings` reclassification. Default
            // severity from our lint pass is Warning.
            let severity = apply_compiler_override(
                &code,
                svn_svelte_compiler::Severity::Warning,
                compiler_overrides,
            );
            let Some(severity) = severity else { continue };
            let key = (code.clone(), path.clone(), w.start_line, w.start_column);
            if !seen.insert(key) {
                continue;
            }
            let href = compiler_code_docs_url(&code, severity);
            diagnostics.push(svn_typecheck::CheckDiagnostic {
                source_path: path.clone(),
                line: w.start_line,
                // LintContext::emit stored 0-based column; CLI adds 1
                // at the source-of-truth boundary (same as bridge).
                column: w.start_column.saturating_add(1),
                end_line: w.end_line,
                end_column: w.end_column.saturating_add(1),
                severity,
                code: svn_typecheck::DiagnosticCode::Slug(code),
                message: w.message,
                source: svn_typecheck::DiagnosticSource::Svelte,
                code_description_url: href,
            });
        }
    }
}

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
        let severity = match severity.trim() {
            "ignore" => CompilerWarningOverride::Ignore,
            "error" => CompilerWarningOverride::Error,
            _ => continue, // upstream drops unrecognized values
        };
        out.insert(code.trim().to_string(), severity);
    }
    out
}

/// Resolve the user's tsconfig path. Honors `--tsconfig` and otherwise
/// walks up from the workspace looking for `tsconfig.json` then
/// `jsconfig.json`. (`--no-tsconfig` is rejected up front in `main` —
/// a tsconfig is required, mirroring `svelte-check --tsgo`.)
///
/// When the resolved tsconfig is a TS project-references solution
/// (`files: []` + no `include` + non-empty `references`), redirect to a
/// sub-project's tsconfig via [`escape_solution_tsconfig`]. Solution
/// files coordinate multiple projects but own no source themselves —
/// our overlay can't inherit useful `paths` / `baseUrl` / resolution
/// settings from one, so extending it leaves every `$lib/*` import
/// unresolved. Common root-of-monorepo case in SvelteKit apps.
/// Returns `(tsconfig_path, escaped_solution)`. `escaped_solution` is
/// `true` only when the resolved path is a sub-project we redirected to
/// from a project-references *solution* config — the one case where the
/// caller should also relocate the workspace. An explicit `--tsconfig`
/// pointing at an ordinary config returns `false`, so the workspace
/// stays put (upstream keeps workspace and tsconfig independent).
fn resolve_tsconfig(workspace: &Path, explicit: Option<&Path>) -> Result<(PathBuf, bool), String> {
    let candidate: PathBuf = if let Some(p) = explicit {
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
        dunce::canonicalize(&resolved).unwrap_or(resolved)
    } else {
        svn_core::walk_up_dirs(workspace, |dir| {
            ["tsconfig.json", "jsconfig.json"]
                .iter()
                .map(|name| dir.join(name))
                .find(|c| c.is_file())
        })
        .ok_or_else(|| {
            format!(
                "no tsconfig.json or jsconfig.json found at or above {}",
                workspace.display()
            )
        })?
    };
    match escape_solution_tsconfig(&candidate) {
        Some(escaped) => Ok((escaped, true)),
        None => Ok((candidate, false)),
    }
}

/// If `candidate` is a solution-style tsconfig, try to redirect to a
/// sub-project's tsconfig that carries real `compilerOptions.paths`.
///
/// Algorithm:
///   1. Parse `candidate`. Return `None` if not a solution.
///   2. For each entry in `references[]`: if the reference points at a
///      file, that IS the sub-project's config (TS references may name
///      any file, not just `tsconfig.json`); if it points at a
///      directory, fall back to the conventional `tsconfig.json` under
///      it.
///   3. Load the referenced config's full extends chain via
///      [`load_chain`]. If any file in the chain declares non-empty
///      `compilerOptions.paths`, return the leaf as the redirect
///      target.
///
/// The extends walk matters in monorepos that declare `paths` once in a
/// shared `tsconfig.base.json` and inherit it into each app; a single
/// `parse_file` of the leaf misses those and leaves us stuck on the
/// solution root with unresolvable `$lib`-style aliases.
///
/// Returns `None` when the tsconfig isn't a solution, no reference's
/// chain declares paths, or any parse fails — keeps the caller's
/// original in those cases.
fn escape_solution_tsconfig(candidate: &Path) -> Option<PathBuf> {
    let parsed = svn_core::tsconfig::parse_file(candidate).ok()?;
    if !parsed.is_solution_style() {
        return None;
    }
    let parent = candidate.parent()?;
    for reference in &parsed.references {
        let ref_path = parent.join(&reference.path);
        let config_path = if ref_path.is_file() {
            // References may name the config file directly (e.g.
            // `./apps/foo/tsconfig.app.json`). The reference's
            // filename is the user's explicit "this is the project
            // config" and we must honor it — a monorepo that picks
            // variant names like `tsconfig.app.json` for runtime code
            // and `tsconfig.node.json` for build-time code would
            // silently redirect to the wrong file (or no file at all)
            // if we hardcoded `tsconfig.json`.
            ref_path
        } else if ref_path.is_dir() {
            let default = ref_path.join("tsconfig.json");
            if !default.is_file() {
                continue;
            }
            default
        } else {
            continue;
        };
        let chain = match svn_core::tsconfig::load_chain(&config_path) {
            Ok(c) => c,
            Err(_) => continue,
        };
        let has_paths = chain.iter().any(|f| {
            f.compiler_options
                .paths
                .as_ref()
                .is_some_and(|p| !p.is_empty())
        });
        if !has_paths {
            continue;
        }
        return Some(dunce::canonicalize(&config_path).unwrap_or(config_path));
    }
    None
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
    solution_root_tsconfig: Option<&Path>,
    tsconfig: &Path,
    output_format: &str,
    threshold: &str,
    fail_on_warnings: bool,
    sources: DiagnosticSources,
    compiler_overrides: &std::collections::HashMap<String, CompilerWarningOverride>,
    color: ColorMode,
    timings: bool,
    tsgo_diagnostics: bool,
    svelte_warnings_mode: SvelteWarningsMode,
    ignore_node_modules_warnings: bool,
    config_resolver: &mut svelte_config::ConfigResolver,
    kit_files_settings: &svn_core::sveltekit::KitFilesSettings,
    include_suggestions: bool,
) -> ExitCode {
    let phase_start = std::time::Instant::now();

    let mark = std::time::Instant::now();
    // TS project scope: honor the tsconfig's `include`/`exclude`
    // patterns so files outside the user's declared project don't get
    // fed into the overlay. Real-world pattern: a monorepo-style
    // project has a root tsconfig whose `include` only covers an app
    // subtree (e.g. `src/renderer/**/*.svelte`) and a separate
    // sub-project `tsconfig.json` with its own paths; upstream
    // svelte-check respects include and silently skips out-of-scope
    // files when run at root, but we used to feed them all to tsgo,
    // firing a cascade of "Cannot find module
    // '$lib/…'" errors because the root tsconfig's `$lib` alias
    // points at the wrong tree for those files.
    let project_scope = svn_core::tsconfig::load_chain(tsconfig).ok().map(|chain| {
        // Resolve include/exclude/files against the config that DECLARES
        // them (TS extends precedence: leaf wins, later array-extends
        // entries beat earlier ones), producing absolute patterns — the
        // same rule the overlay builder uses. `None` = the field is
        // declared nowhere in the chain; a declared-but-empty include
        // compiles to an empty glob set that matches nothing (TS
        // replace-on-child: `"include": []` admits no files).
        let include =
            discovery::resolve_patterns_against_declaring_dir(&chain, |f| f.include.as_deref())
                .map(|pats| {
                    discovery::build_glob_set_absolute(&pats)
                        .unwrap_or_else(globset::GlobSet::empty)
                });
        let exclude =
            discovery::resolve_patterns_against_declaring_dir(&chain, |f| f.exclude.as_deref())
                .map(|pats| {
                    discovery::build_glob_set_absolute(&pats)
                        .unwrap_or_else(globset::GlobSet::empty)
                });
        // Files explicitly listed in tsconfig's `files` field bypass
        // both `include` glob matching AND `exclude` filtering (TS
        // spec: https://www.typescriptlang.org/tsconfig/#exclude —
        // "A file specified by exclude can still become part of your
        // codebase due to … being specified in the files list").
        // Canonicalized because the matcher compares against canonical
        // walker paths.
        let explicit_files: std::collections::HashSet<PathBuf> =
            discovery::resolve_patterns_against_declaring_dir(&chain, |f| f.files.as_deref())
                .unwrap_or_default()
                .into_iter()
                .map(PathBuf::from)
                .filter_map(|p| dunce::canonicalize(&p).ok().or(Some(p)))
                .collect();
        (include, exclude, explicit_files)
    });
    let in_project_scope = |path: &Path| -> bool {
        let Some((include, exclude, files)) = &project_scope else {
            return true;
        };
        // `files` bypasses exclude. Walker paths are already canonical
        // because `workspace` was canonicalized at startup
        // (`main.rs:180`), so the lookup matches the canonical form
        // built into the set above without re-canonicalizing per file.
        // Limitation: a symlinked directory inside the workspace tree
        // could yield a walker path whose canonical form is elsewhere;
        // unobserved in real Svelte projects and not worth the
        // per-entry stat cost (~1100 calls × ~30µs each on 1k-file
        // workspaces).
        if files.contains(path) {
            return true;
        }
        // TS spec: when `files` is non-empty AND `include` is absent,
        // ONLY entries listed in `files` are in the project (closed-
        // world). Without this guard we'd default `include = match all`
        // and pull every walked file into scope — wrong for the
        // explicit-allowlist tsconfig pattern. Mirrors upstream
        // svelte-check + tsc's project-membership rules.
        if include.is_none() && !files.is_empty() {
            return false;
        }
        // Patterns are now absolute (resolved against the declaring
        // config's dir), so match the canonical absolute walker path
        // directly — no workspace-relativization needed.
        let included = include.as_ref().is_none_or(|set| set.is_match(path));
        let excluded = exclude.as_ref().is_some_and(|set| set.is_match(path));
        included && !excluded
    };
    let (svelte_files_raw, kit_files_raw, runes_modules_raw, user_scripts_raw) =
        discovery::discover_relevant_files_with_settings(workspace, kit_files_settings);
    // Svelte-file emit: we walk ALL discovered `.svelte` files, not
    // just the in-scope subset. An out-of-scope file might be
    // imported by an in-scope one — upstream's LanguageService
    // follows that import and type-checks the target. For us to
    // match, tsgo needs to find an overlay for the target; that's
    // what the SvelteAuxiliary kind provides (writes the overlay +
    // ambient sidecar, but doesn't list the path in the overlay
    // tsconfig's `files`, so the out-of-scope file is only
    // type-checked if something in scope reaches it).
    let svelte_files_all: Vec<PathBuf> = svelte_files_raw;
    let svelte_files: Vec<PathBuf> = svelte_files_all
        .iter()
        .filter(|p| in_project_scope(p))
        .cloned()
        .collect();
    let svelte_files_aux: Vec<PathBuf> = svelte_files_all
        .iter()
        .filter(|p| !in_project_scope(p))
        .cloned()
        .collect();
    // Kit files (route modules, hooks, params). We don't type-check
    // them ourselves — tsgo processes them via regular `.ts` include —
    // so only in-scope kit files feed the inject pass below. The FULL
    // discovered list (`kit_files_raw`) still counts toward the
    // COMPLETED denominator: upstream's findFiles enumeration has no
    // tsconfig scoping.
    let kit_files: Vec<PathBuf> = kit_files_raw
        .iter()
        .filter(|p| in_project_scope(p))
        .cloned()
        .collect();
    // `.svelte.ts` runes-module set. Walker paths are canonical
    // (workspace is canonicalized at startup, `main.rs:180`), so
    // dropping the per-entry canonicalize here costs nothing as long
    // as the consumer at `rewrite_svelte_imports_for_collisions`
    // canonicalizes its probe paths the same way (it does — the
    // sibling-runes probe still calls `dunce::canonicalize`, which
    // resolves any `./` / `..` from a relative import specifier into
    // the same canonical form held in this set).
    let runes_modules_set: std::collections::HashSet<PathBuf> =
        runes_modules_raw.into_iter().collect();
    let user_script_files: Vec<PathBuf> = user_scripts_raw
        .into_iter()
        .filter(|p| in_project_scope(p))
        .collect();
    // Resolve each file's nearest svelte.config (warningFilter / runes)
    // now, sequentially — the parallel lint pass below only does
    // read-only lookups. Nested configs whose warningFilter didn't
    // fully translate get the same stderr notice as the root config.
    config_resolver.prime(svelte_files_all.iter());
    for (cfg_path, plan) in config_resolver.nested_partial_configs() {
        warn_partial_warning_filter(cfg_path, plan);
    }
    let t_discovery = mark.elapsed();

    // Read every source up-front; we need the bytes for both the
    // tsgo-typecheck path and the svelte/compiler bridge.
    //
    // `svelte_sources` contains in-scope + auxiliary entries back to
    // back. Entries up to `svelte_sources_in_scope_end` are listed in
    // the overlay tsconfig's `files` (and run through the compiler
    // warning bridge); the tail past that are auxiliary overlays
    // that only exist so tsgo's import-following can reach them.
    let mut svelte_sources: Vec<(PathBuf, String)> =
        Vec::with_capacity(svelte_files.len() + svelte_files_aux.len());
    for file in &svelte_files {
        match std::fs::read_to_string(file) {
            Ok(s) => svelte_sources.push((file.clone(), s)),
            Err(err) => {
                eprintln!("failed to read {}: {err}", file.display());
            }
        }
    }
    let svelte_sources_in_scope_end = svelte_sources.len();
    for file in &svelte_files_aux {
        match std::fs::read_to_string(file) {
            Ok(s) => svelte_sources.push((file.clone(), s)),
            Err(err) => {
                eprintln!("failed to read {}: {err}", file.display());
            }
        }
    }

    let mark = std::time::Instant::now();
    // The whole parse → analyze → emit + kit-inject + collision-
    // rewrite pipeline only feeds tsgo. When --diagnostic-sources
    // excludes `js`, tsgo is skipped entirely, so this work would
    // be discarded — gate it on `sources.js` to skip it up front.
    // The svelte/compiler bridge below still runs (it consumes
    // `svelte_sources`, not `inputs`).
    let mut inputs: Vec<svn_typecheck::CheckInput> = Vec::new();
    if sources.js {
        // Per-file parse → analyze → emit is pure compute with no shared
        // mutable state (each iteration owns its own oxc Allocator inside
        // the called functions). rayon distributes across the thread pool
        // and `collect_into_vec` preserves source order so the resulting
        // `inputs` matches `svelte_sources` index-for-index.
        inputs.reserve(svelte_sources.len());
        svelte_sources
            .par_iter()
            .enumerate()
            .map(|(idx, (file, source))| {
                let (doc, _parse_errors) = svn_parser::parse_sections(source);
                let (fragment, _template_errors) =
                    svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
                let summary = svn_analyze::walk_template(&fragment, source);
                // Overlay extension mirrors upstream svelte-check's
                // `isTsSvelte(text)` per-file dispatch
                // (`language-tools/packages/svelte-check/src/incremental.ts:213`):
                // `<script lang="ts">` → `.svelte.svn.ts` with TS-strict
                // inference; otherwise `.svelte.svn.js`, which flips tsgo
                // into JS-loose inference (`$state([])` → `any[]`;
                // `noImplicitAny:false` defaults) and lets tsgo natively
                // parse user-authored JSDoc `@typedef` / `@type`
                // annotations on Svelte-4 `export let` props.
                let is_ts = doc.script_lang() == svn_parser::ScriptLang::Ts;
                let emitted =
                    svn_emit::emit_document_with_lang(&doc, &fragment, &summary, file, is_ts);
                let kind = if idx < svelte_sources_in_scope_end {
                    svn_typecheck::InputKind::Svelte
                } else {
                    svn_typecheck::InputKind::SvelteAuxiliary
                };
                svn_typecheck::CheckInput {
                    source_path: file.clone(),
                    generated_ts: emitted.typescript,
                    line_map: emitted.line_map,
                    token_map: emitted.token_map,
                    overlay_line_starts: emitted.overlay_line_starts,
                    source_line_starts: emitted.source_line_starts,
                    kind,
                    is_ts_overlay: is_ts,
                }
            })
            .collect_into_vec(&mut inputs);

        // Kit files (`+server.ts`, `+page.ts`, hooks, params): run them
        // through the inject pass to splice in `$types` imports so the
        // user's handler destructures (`{url}` / `{request}` / …)
        // type-check against `RequestEvent` / `LoadEvent` / etc. If
        // `inject` returns `None` (no handlers matched), skip — the
        // file type-checks as the user wrote it and the original path
        // stays in tsgo's program via the normal `include` glob.
        inputs.extend(kit_files.iter().filter_map(|file| {
            let source = std::fs::read_to_string(file).ok()?;
            let generated = svn_emit::kit_inject::inject(file, &source)?;
            Some(svn_typecheck::CheckInput {
                source_path: file.clone(),
                generated_ts: generated,
                line_map: Vec::new(),
                token_map: Vec::new(),
                overlay_line_starts: Vec::new(),
                source_line_starts: Vec::new(),
                kind: svn_typecheck::InputKind::KitFile,
                is_ts_overlay: true,
            })
        }));

        // User-`.ts`-overlay for the sibling-collision case: when a user
        // `.ts` file imports `./Foo.svelte` where `Foo.svelte.ts` exists
        // as sibling, tsgo's `rootDirs` resolution picks the user's source
        // tree (longest matching prefix), then auto-extends `.svelte` to
        // `.svelte.ts` and lands on the runes module — which has named
        // exports but no `default`, firing TS2305. Rewriting the import
        // specifier to `.svelte.svn.js` in an overlay sidesteps the
        // auto-extension entirely; tsgo resolves via bundler module
        // resolution straight to the cache-side `.svelte.svn.ts`.
        //
        // Scope: both plain user `.ts` files AND `.svelte.ts` runes
        // modules themselves — a `Foo.svelte.ts` module can import a
        // sibling-collision `./Bar.svelte` (where `Bar.svelte.ts` also
        // exists), and that specifier has the same resolution bug. No
        // current bench exercises the `.svelte.ts` → collision-sibling
        // path, but handling it here completes the pattern.
        //
        // Only files that actually contain a collision-case import get an
        // overlay; others pass through tsgo's regular include. Fast-path
        // skip when no runes modules were discovered.
        if !runes_modules_set.is_empty() {
            let rewrite_candidates = user_script_files.iter().chain(runes_modules_set.iter());
            inputs.extend(rewrite_candidates.filter_map(|file| {
                let source = std::fs::read_to_string(file).ok()?;
                let rewritten =
                    rewrite_svelte_imports_for_collisions(file, &source, &runes_modules_set)?;
                Some(svn_typecheck::CheckInput {
                    source_path: file.clone(),
                    generated_ts: rewritten,
                    line_map: Vec::new(),
                    token_map: Vec::new(),
                    overlay_line_starts: Vec::new(),
                    source_line_starts: Vec::new(),
                    kind: svn_typecheck::InputKind::UserTsOverlay,
                    is_ts_overlay: true,
                })
            }));
        }
    }
    let t_emit = mark.elapsed();

    // Run tsgo (`js`/`ts` source). Skipped entirely when
    // `--diagnostic-sources` opts out of `js`. Move `inputs` into the
    // call so each `generated_ts` string drops as soon as it has been
    // written to the cache — see svn_typecheck::check docs.
    let mark = std::time::Instant::now();
    let (mut diagnostics, tsgo_diag_block) = if sources.js {
        match svn_typecheck::check(
            workspace,
            solution_root_tsconfig,
            tsconfig,
            inputs,
            tsgo_diagnostics,
            include_suggestions,
        ) {
            Ok(out) => (out.diagnostics, out.extended_diagnostics),
            Err(err) => {
                let message = format!("type-check failed: {err}");
                eprintln!("svelte-check-native: {message}");
                // Machine consumers key off a FAILURE line; emit one so
                // a fatal check error isn't a silent stop on stdout.
                print_machine_failure(output_format, &message);
                return ExitCode::from(2);
            }
        }
    } else {
        // When tsgo is skipped, drop `inputs` early too so we don't
        // hold the generated TS strings through the bridge phase.
        drop(inputs);
        (Vec::new(), None)
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
    // Files with a fatal Svelte parse error (native mode). Their tsgo
    // overlay is garbage, so we drop tsgo's diagnostics for them below
    // and report only the syntax error.
    let mut broken_files: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    if sources.svelte {
        svelte_sources.truncate(svelte_sources_in_scope_end);

        // Track which (code, path, offset) tuples we've already
        // pushed so `--svelte-warnings=both` can dedup bridge/native
        // overlap without double-counting.
        let mut seen: std::collections::HashSet<(String, PathBuf, u32, u32)> =
            std::collections::HashSet::new();

        let run_native = matches!(svelte_warnings_mode, SvelteWarningsMode::Native);
        if run_native {
            // Fatal compile diagnostics (syntax errors → marked broken so
            // their tsgo noise is dropped below; analyze-phase structural
            // errors like gh#30's const placement → not broken, tsgo still
            // runs) AND lint warnings, all from a single parse per file.
            emit_native_svelte_diagnostics(
                &svelte_sources,
                compiler_overrides,
                &mut diagnostics,
                &mut seen,
                workspace,
                config_resolver,
                &mut broken_files,
            );
        }

        let run_bridge = matches!(svelte_warnings_mode, SvelteWarningsMode::Bridge);
        if run_bridge {
            match svn_svelte_compiler::compile_batch(workspace, std::mem::take(&mut svelte_sources))
            {
                Ok(per_file) => {
                    for (path, warnings) in per_file {
                        for w in warnings {
                            let severity =
                                apply_compiler_override(&w.code, w.severity, compiler_overrides);
                            let Some(severity) = severity else { continue };
                            let href = compiler_code_docs_url(&w.code, severity);
                            let key = (w.code.clone(), path.clone(), w.start.line, w.start.column);
                            if !seen.insert(key) {
                                continue;
                            }
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
                                message: w.message,
                                source: svn_typecheck::DiagnosticSource::Svelte,
                                code_description_url: href,
                            });
                        }
                    }
                }
                Err(_) => {
                    // Bridge unavailable — proceed with TS diagnostics only.
                }
            }
        }
    }
    // Drop tsgo's diagnostics for files we flagged with a fatal parse
    // error — their overlay was generated from a recovered (garbage)
    // AST, so any TS diagnostic on them is noise. Upstream reports only
    // the parse error for such files. The Svelte-source parse error
    // itself is retained.
    if !broken_files.is_empty() {
        diagnostics.retain(|d| {
            matches!(d.source, svn_typecheck::DiagnosticSource::Svelte)
                || !broken_files.contains(&d.source_path)
        });
    }
    let t_compiler = mark.elapsed();

    // `css` source is reserved — once we add a CSS linter (or wire one
    // through preprocessor output), this is where we'd run it. For now
    // the flag's effect on `css` is purely opt-out semantics; opting in
    // is a no-op until we have something to emit.
    let _ = sources.css;

    // `--ignore-node-modules-warnings`: drop every Svelte-source
    // warning whose path contains a `node_modules/` component.
    // Mirrors upstream's common `warningFilter: (w) => !w.filename?.
    // includes('node_modules')` pattern (19/100 sampled real-world
    // uses — see notes/lint-progress.md Tier-1 section). Only
    // affects Svelte diagnostics; TS/JS diagnostics fall through
    // because tsgo's `include`/`exclude` already own that boundary.
    if ignore_node_modules_warnings {
        diagnostics.retain(|d| {
            !matches!(d.source, svn_typecheck::DiagnosticSource::Svelte)
                || !path_is_under_node_modules(&d.source_path)
        });
    }

    // Tier 2: apply the drop rules we statically translated from each
    // file's NEAREST `svelte.config.js` `warningFilter` (per-document
    // resolution, mirroring upstream's configLoader). Applies only to
    // the Svelte diagnostic source — TS/JS diagnostics are tsgo's
    // domain.
    diagnostics.retain(|d| {
        if !matches!(d.source, svn_typecheck::DiagnosticSource::Svelte) {
            return true;
        }
        let plan = &config_resolver.for_path(&d.source_path).warning_filter_plan;
        if plan.rules.is_empty() && plan.constant.is_none() {
            return true;
        }
        let code = match &d.code {
            svn_typecheck::DiagnosticCode::Slug(s) => s.as_str(),
            svn_typecheck::DiagnosticCode::Numeric(_) => "",
        };
        !plan.should_drop(code, Some(&d.source_path))
    });

    // NOTE: `--threshold error` is a PRINT-TIME filter only — applied
    // per-diagnostic inside `print_diagnostics`. The counts and exit
    // code below are computed from the FULL set, mirroring upstream
    // (the COMPLETED line reports the true warning count and
    // `--fail-on-warnings` still fires). Filtering here instead would
    // zero out `warning_count` and make `--threshold error
    // --fail-on-warnings` exit 0 with warnings present.
    let error_count = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Error))
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|d| matches!(d.severity, svn_typecheck::Severity::Warning))
        .count();

    // `<N> FILES` in the COMPLETED line mirrors upstream svelte-check's
    // denominator exactly: `|entries ∪ files-with-diagnostics|`
    // (index.ts `writeDiagnostics` over the map
    // `getSvelteDiagnosticsForIncremental` seeds), where `entries` is
    // every `.svelte` + SvelteKit "Kit file" (route modules like
    // `+page.ts`, hooks, params) that `findFiles` discovers
    // WORKSPACE-WIDE — only node_modules/dot-dir filtering, no tsconfig
    // `include`/`exclude` scoping (incremental.ts deliberately doesn't
    // use them to filter virtualized files) — and files-with-diagnostics
    // adds any NON-entry file that picked up a TS diagnostic at tsgo
    // time (typically `tsconfig.json`-level errors). The entry half
    // exists only while a `svelte` or `css` source is enabled: with
    // js-only sources upstream's per-entry seeding early-returns
    // (index.ts:324-330) and the count collapses to just the
    // diagnostic-bearing files. Both sets deduplicated against
    // source_path.
    let files_for_completed: usize = {
        use std::collections::HashSet;
        let mut seen: HashSet<&Path> = HashSet::new();
        if sources.svelte || sources.css {
            seen.extend(svelte_files_all.iter().map(PathBuf::as_path));
            seen.extend(kit_files_raw.iter().map(PathBuf::as_path));
        }
        for d in &diagnostics {
            seen.insert(d.source_path.as_path());
        }
        seen.len()
    };
    print_diagnostics(
        workspace,
        &diagnostics,
        output_format,
        color,
        files_for_completed,
        threshold,
    );

    // `--tsgo-diagnostics` block — printed to stderr so machine-output
    // consumers parsing stdout (editors, CI wrappers) don't have to
    // skip past perf stats. Same stream choice as `--timings`.
    if let Some(block) = tsgo_diag_block.as_deref() {
        eprintln!();
        eprintln!("tsgo --extendedDiagnostics");
        eprintln!("{block}");
    }

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

/// `--emit-ts` flow: discover `.svelte` files, parse, emit, print to stdout
/// with file separators. Exits 0 unconditionally — debug-mode is best-effort.
fn run_emit_ts(workspace: &Path) -> ExitCode {
    // Honour svelte config `namespace: 'foreign'` (preserve attribute
    // case) in the debug-emit path too, mirroring the check path.
    if let Some(cfg) = svelte_config::find_svelte_config(workspace) {
        svn_emit::set_preserve_attribute_case(svelte_config::analyse(&cfg).preserve_attribute_case);
    }
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
        let is_ts = doc.script_lang() == svn_parser::ScriptLang::Ts;
        let emitted = svn_emit::emit_document_with_lang(&doc, &fragment, &summary, file, is_ts);
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

#[cfg(test)]
mod tests {
    use super::*;
    use svn_typecheck::Severity;

    // ---- CLI flag surface ----

    #[test]
    fn config_flag_is_parsed() {
        // `--config <path>` (upstream #3031/#3066) is accepted and
        // captured for Svelte-config override.
        let cli = Cli::try_parse_from(["svelte-check-native", "--config", "my.svelte.config.js"])
            .expect("--config should parse");
        assert_eq!(
            cli.config.as_deref(),
            Some(Path::new("my.svelte.config.js"))
        );
    }

    #[test]
    fn tsgo_flag_is_accepted_as_noop() {
        // We are definitionally tsgo-powered; `--tsgo` is accepted for
        // drop-in compat but selects nothing.
        let cli = Cli::try_parse_from(["svelte-check-native", "--tsgo"]).expect("--tsgo parses");
        assert!(cli.tsgo);
    }

    #[test]
    fn tsgo_experimental_api_flag_is_accepted_as_noop() {
        // Upstream svelte-check's `test-sanity.js` (our `upstream_sanity`
        // gate) runs the binary with `--tsgo-experimental-api` and
        // expects identical output. We always use tsgo, so it's a no-op
        // accept that produces the same diagnostics as the default path.
        let cli = Cli::try_parse_from(["svelte-check-native", "--tsgo-experimental-api"])
            .expect("--tsgo-experimental-api should parse");
        assert!(cli.tsgo_experimental_api);
    }

    // ---- parse_diagnostic_sources ----

    #[test]
    fn diagnostic_sources_default_no_css() {
        let s = parse_diagnostic_sources(None).unwrap();
        assert!(s.js && s.svelte && !s.css);
    }

    #[test]
    fn diagnostic_sources_explicit_ts_svelte() {
        let s = parse_diagnostic_sources(Some("ts,svelte")).unwrap();
        assert!(s.js && s.svelte && !s.css);
    }

    #[test]
    fn diagnostic_sources_css_accepted_no_op() {
        // Round 2 #1: css/scss/sass/less/postcss accept silently
        // (was a hard error pre-2026-04-26). Many monorepo CI scripts
        // pass `ts,svelte,css` template-fashion; rejecting outright
        // blocked migrations from upstream svelte-check.
        let s = parse_diagnostic_sources(Some("ts,svelte,css")).unwrap();
        assert!(s.js && s.svelte && s.css);
        // scss / sass / less / postcss aliases all route to css.
        for alias in ["scss", "sass", "less", "postcss"] {
            let s = parse_diagnostic_sources(Some(alias)).unwrap();
            assert!(s.css, "{alias} should set css=true");
        }
    }

    #[test]
    fn diagnostic_sources_unknown_entry_warns_not_errors() {
        // Unknown entries print to stderr and continue (no Err return).
        let s = parse_diagnostic_sources(Some("ts,frobnicate,svelte")).unwrap();
        assert!(s.js && s.svelte);
    }

    #[test]
    fn compiler_docs_url_routes_warning_to_compiler_warnings_anchor() {
        assert_eq!(
            compiler_code_docs_url("state_referenced_locally", Severity::Warning),
            Some(
                "https://svelte.dev/docs/svelte/compiler-warnings#state_referenced_locally"
                    .to_string()
            ),
        );
    }

    #[test]
    fn compiler_docs_url_routes_error_to_compiler_errors_anchor() {
        // Bridge-emitted compile-error codes (parse errors, etc.)
        // go to the compiler-errors page, not warnings.
        assert_eq!(
            compiler_code_docs_url("compile_error", Severity::Error),
            Some("https://svelte.dev/docs/svelte/compiler-errors#compile_error".to_string()),
        );
    }

    #[test]
    fn compiler_docs_url_normalizes_hyphens_to_underscores() {
        // svelte.dev's anchor slugs use underscores; upstream
        // svelte-check normalizes hyphenated codes before joining.
        assert_eq!(
            compiler_code_docs_url("element-invalid-self-closing-tag", Severity::Warning),
            Some(
                "https://svelte.dev/docs/svelte/compiler-warnings#element_invalid_self_closing_tag"
                    .to_string()
            ),
        );
    }

    #[test]
    fn compiler_docs_url_skips_codes_without_separator() {
        // Single-word codes like "unknown" aren't documented
        // anchors; upstream's filter requires at least one `_` or `-`.
        assert_eq!(compiler_code_docs_url("unknown", Severity::Warning), None);
    }

    #[test]
    fn compiler_docs_url_skips_uppercase_first_char() {
        // Upstream filters out codes whose first char isn't a lower
        // ASCII letter (rules out TS numeric codes, PascalCase, etc.).
        assert_eq!(compiler_code_docs_url("TS2322", Severity::Warning), None);
        assert_eq!(
            compiler_code_docs_url("PascalCase_code", Severity::Warning),
            None,
        );
    }

    #[test]
    fn compiler_docs_url_skips_empty_code() {
        assert_eq!(compiler_code_docs_url("", Severity::Warning), None);
    }

    #[test]
    fn compiler_docs_url_accepts_hyphen_only_code() {
        // Codes with `-` but no `_` still pass the filter; normalize
        // to underscore in the URL.
        assert_eq!(
            compiler_code_docs_url("a11y-autofocus", Severity::Warning),
            Some("https://svelte.dev/docs/svelte/compiler-warnings#a11y_autofocus".to_string()),
        );
    }

    /// Write a tsconfig with the given JSON body and return its path.
    fn write_tsconfig(dir: &Path, name: &str, body: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        path
    }

    #[test]
    fn escape_solution_keeps_referenced_file_name_not_just_dir() {
        // Reference points at a variant filename like tsconfig.app.json.
        // Pre-fix we'd drop the filename and try <dir>/tsconfig.json,
        // miss it, and never redirect — leaving the user stuck on the
        // solution root with unresolvable paths.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let app_dir = root.join("apps/foo");
        std::fs::create_dir_all(&app_dir).unwrap();
        write_tsconfig(
            root,
            "tsconfig.json",
            r#"{ "files": [], "references": [{ "path": "./apps/foo/tsconfig.app.json" }] }"#,
        );
        let app_ts = write_tsconfig(
            &app_dir,
            "tsconfig.app.json",
            r#"{ "compilerOptions": { "paths": { "$lib/*": ["./src/lib/*"] } } }"#,
        );
        let redirected = escape_solution_tsconfig(&root.join("tsconfig.json")).unwrap();
        assert_eq!(
            dunce::canonicalize(&redirected).unwrap(),
            dunce::canonicalize(&app_ts).unwrap(),
        );
    }

    #[test]
    fn escape_solution_follows_extends_for_paths_discovery() {
        // Leaf `tsconfig.json` declares no paths of its own but inherits
        // them from a shared `tsconfig.base.json` via `extends`. Pre-fix
        // we only looked at the leaf and missed the redirect entirely.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        write_tsconfig(
            root,
            "tsconfig.base.json",
            r#"{ "compilerOptions": { "paths": { "$app/*": ["./src/app/*"] } } }"#,
        );
        let app_dir = root.join("apps/foo");
        std::fs::create_dir_all(&app_dir).unwrap();
        write_tsconfig(
            root,
            "tsconfig.json",
            r#"{ "files": [], "references": [{ "path": "./apps/foo" }] }"#,
        );
        let leaf = write_tsconfig(
            &app_dir,
            "tsconfig.json",
            r#"{ "extends": "../../tsconfig.base.json", "compilerOptions": { "strict": true } }"#,
        );
        let redirected = escape_solution_tsconfig(&root.join("tsconfig.json")).unwrap();
        assert_eq!(
            dunce::canonicalize(&redirected).unwrap(),
            dunce::canonicalize(&leaf).unwrap(),
        );
    }

    #[test]
    fn escape_solution_skips_reference_whose_chain_has_no_paths() {
        // References that inherit nothing path-related stay on the
        // solution root. The escape only exists to rescue paths
        // resolution; skipping leaves other flows untouched.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let app_dir = root.join("apps/foo");
        std::fs::create_dir_all(&app_dir).unwrap();
        write_tsconfig(
            root,
            "tsconfig.json",
            r#"{ "files": [], "references": [{ "path": "./apps/foo" }] }"#,
        );
        write_tsconfig(
            &app_dir,
            "tsconfig.json",
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        assert!(escape_solution_tsconfig(&root.join("tsconfig.json")).is_none());
    }

    #[test]
    fn escape_solution_returns_none_for_non_solution_tsconfig() {
        let tmp = tempfile::tempdir().unwrap();
        let ts = write_tsconfig(
            tmp.path(),
            "tsconfig.json",
            r#"{ "compilerOptions": { "strict": true }, "include": ["src/**/*"] }"#,
        );
        assert!(escape_solution_tsconfig(&ts).is_none());
    }
}
