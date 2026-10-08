//! Spawn tsgo against a populated cache and parse its output.
//!
//! The orchestrator (in `lib.rs`) populates the cache with generated
//! `.svelte.ts` files and writes the overlay tsconfig; the runner then
//! invokes tsgo and converts its stdout into a [`RunOutput`] of
//! diagnostics.
//!
//! Invocation:
//!
//! ```text
//! tsgo --project <overlay.json> --pretty true --noErrorTruncation [--extendedDiagnostics]
//! ```
//!
//! `--pretty true` and `--noErrorTruncation` mirror upstream svelte-check's
//! invocation. `--extendedDiagnostics` is added when the user passes
//! `--tsgo-diagnostics`; its stats block (file/line/symbol counts, memory
//! use, phase timings) is captured and returned for the CLI to print.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use crate::discovery::TsgoBinary;
use crate::output::{RawDiagnostic, parse as parse_output};

/// Default wall-clock cap on a single tsgo invocation. Generous — a
/// real check of a large monorepo finishes well inside this — but
/// bounded so a hung/deadlocked tsgo can't hang us forever. Override
/// with `SVN_TSGO_TIMEOUT_SECS` (set to `0` to disable the cap).
const DEFAULT_TSGO_TIMEOUT_SECS: u64 = 600;

/// Errors when running tsgo.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("failed to spawn tsgo: {0}")]
    Spawn(#[source] std::io::Error),
    /// The compiler did not complete a run: it was killed by a signal,
    /// or exited non-zero without printing a diagnostic we could parse
    /// (see [`run_failed`]). Reporting either as a clean run is the
    /// worst failure mode for a checker (false-clean in CI).
    #[error(
        "The TypeScript compiler process {}.{}",
        match .code {
            Some(c) => format!("exited with code {c} without a parseable diagnostic"),
            None => "was killed by a signal".into(),
        },
        if .output.is_empty() { String::new() } else { format!("\n{}", .output) }
    )]
    Failed { code: Option<i32>, output: String },
    /// tsgo did not finish within the configured timeout and was killed.
    #[error("tsgo timed out after {}s and was killed", .0.as_secs())]
    Timeout(Duration),
    /// The compiler reported diagnostics, every one of them was filtered
    /// out as overlay noise, and nothing was left to show. Reporting that
    /// as a clean run is the worst thing this tool can do — a config
    /// error aborts the whole program, so "0 errors" would mean "nothing
    /// was checked", and in CI the two look identical.
    #[error("{0}")]
    AllDiagnosticsFiltered(String),
}

/// Did the compiler fail to complete the run? svelte-check's rule
/// (`incremental.ts` `runTypeScriptDiagnostics`): death by a signal
/// (`None` on Unix) always fails, since the run did not finish and
/// anything it printed is partial. A non-zero exit fails only when
/// nothing parseable came out, because the compilers also exit non-zero
/// just to say they reported diagnostics — and the engines disagree on
/// the code for that, so no specific code can be trusted.
fn run_failed(code: Option<i32>, parsed_any: bool) -> bool {
    match code {
        None => true,
        Some(0) => false,
        Some(_) => !parsed_any,
    }
}

/// What `run` returns: the parsed diagnostics and an optional
/// extended-diagnostics block.
#[derive(Debug)]
pub struct RunOutput {
    pub diagnostics: Vec<RawDiagnostic>,
    /// True when the compiler exited non-zero — i.e. it reported at
    /// least one diagnostic (exit 1), or failed outright (2+/signal).
    /// Exit 0 is the only "nothing to say" status.
    ///
    /// The caller needs this to notice that the compiler had something
    /// to report and our filters then swallowed all of it. Note the
    /// engines disagree on the code for a fatal config error —
    /// `@typescript/native-preview` exits 2, `typescript@7`'s `tsc`
    /// exits 1 — so only "non-zero" is portable, not any specific code.
    pub nonzero_exit: bool,
    /// Every file tsgo loaded (`--listFiles`), in its order.
    pub program_files: Vec<PathBuf>,
    /// `--extendedDiagnostics` block captured verbatim from tsgo's
    /// stdout tail. `Some(text)` iff the caller requested extended
    /// diagnostics AND tsgo emitted a recognizable block. Text is the
    /// trailing lines starting from the first `Files:` label and
    /// running through tsgo's final `Total time:` line.
    pub extended_diagnostics: Option<String>,
}

/// Run tsgo against an overlay tsconfig. Returns the parsed diagnostics
/// and an optional extended-diagnostics block.
///
/// `workspace` is set as tsgo's working directory so the diagnostic paths
/// it emits (which are relative to its cwd) resolve under the workspace
/// root via `workspace.join()`. Without this, running the binary from a
/// monorepo root with `--workspace ./apps/admin` produces phantom paths
/// like `apps/admin/apps/admin/.svelte-check/tsconfig.json`, which
/// breaks the overlay-noise filter's path match so structural overlay
/// artifacts leak as user-visible errors.
///
/// When `extended_diagnostics` is true, `--extendedDiagnostics` is
/// appended to tsgo's argv; the stats block tsgo emits after the last
/// diagnostic is captured in the returned `extended_diagnostics` field.
///
/// When `include_suggestions` is true, `--noUnusedLocals` and
/// `--noUnusedParameters` are appended to tsgo's argv so TS6133
/// (declared-but-never-read) fires in CLI mode the way upstream LS's
/// `getSuggestionDiagnostics` would. The caller is responsible for
/// reclassifying the resulting codes to `Severity::Hint` afterwards;
/// the runner just gets tsgo to emit them.
pub fn run(
    tsgo: &TsgoBinary,
    overlay_tsconfig: &Path,
    workspace: &Path,
    extended_diagnostics: bool,
    include_suggestions: bool,
) -> Result<RunOutput, RunError> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--project".into(),
        overlay_tsconfig.into(),
        "--pretty".into(),
        "true".into(),
        "--noErrorTruncation".into(),
        // The program's files, for the replay fingerprint.
        "--listFiles".into(),
    ];
    if extended_diagnostics {
        args.push("--extendedDiagnostics".into());
    }
    if include_suggestions {
        args.push("--noUnusedLocals".into());
        args.push("--noUnusedParameters".into());
    }
    // TS 7.0 parallelism knobs, exposed via env vars while we
    // validate impact. Eventually become first-class CLI flags on
    // `svelte-check-native`. See `notes/ts7-tracking.md`.
    //
    // `SVN_TSGO_BUILDERS` is intentionally NOT plumbed here.
    // `--builders` is a `tsgo --build` (project-references) flag;
    // our single-project invocation mode (`--project <overlay>`)
    // treats it as TS5093 and exits without diagnostics. Users
    // who set the env var would silently get 0-error runs across
    // their whole workspace. If we ever switch to `--build` mode
    // the flag comes back here, not before.
    if let Ok(n) = std::env::var("SVN_TSGO_CHECKERS")
        && !n.is_empty()
    {
        args.push("--checkers".into());
        args.push(n.into());
    }
    if std::env::var("SVN_TSGO_SINGLE_THREADED").is_ok_and(|v| !v.is_empty()) {
        args.push("--singleThreaded".into());
    }

    let mut cmd = if tsgo.needs_node {
        let mut c = Command::new("node");
        c.arg(&tsgo.path);
        c.args(&args);
        c
    } else {
        let mut c = Command::new(&tsgo.path);
        c.args(&args);
        c
    };
    cmd.current_dir(workspace);
    // The compiler is a short-lived Go process whose parse phase
    // allocates in one large burst; with default GC pacing a
    // significant share of that phase goes to the collector returning
    // and re-faulting heap pages mid-run (~10% of total wall time
    // measured via pprof on a 8.5k-file program: Parse 0.43s → 0.33s
    // with GC off). Disable GC for the process lifetime and cap the
    // heap with a memory limit as the safety valve — at the limit the
    // Go runtime re-enables collection instead of aborting. A
    // user-provided value for either var wins: someone constraining
    // memory on a small CI box knows better than our default.
    if std::env::var_os("GOGC").is_none() && std::env::var_os("GOMEMLIMIT").is_none() {
        cmd.env("GOGC", "off");
        cmd.env("GOMEMLIMIT", "4GiB");
    }
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let child = cmd.spawn().map_err(RunError::Spawn)?;
    let timeout = tsgo_timeout();
    let Wait {
        status,
        stdout: stdout_bytes,
        stderr: stderr_bytes,
        timed_out,
    } = wait_with_timeout(child, timeout).map_err(RunError::Spawn)?;

    if timed_out {
        return Err(RunError::Timeout(timeout));
    }

    let stdout = String::from_utf8_lossy(&stdout_bytes);
    let stderr = String::from_utf8_lossy(&stderr_bytes);

    let extended_diag_text = if extended_diagnostics {
        extract_extended_diagnostics(&stdout)
    } else {
        None
    };

    let mut combined = String::new();
    combined.push_str(&stdout);
    combined.push('\n');
    combined.push_str(&stderr);

    let diagnostics = parse_output(&combined);

    if run_failed(status.code(), !diagnostics.is_empty()) {
        return Err(RunError::Failed {
            code: status.code(),
            output: failure_output(&stdout, &stderr),
        });
    }

    Ok(RunOutput {
        diagnostics,
        extended_diagnostics: extended_diag_text,
        nonzero_exit: !matches!(status.code(), Some(0)),
        program_files: listed_files(&stdout),
    })
}

/// The `--listFiles` lines of tsgo's output: one absolute path per
/// line. Diagnostic headers and code frames never form a bare
/// absolute path of an existing file.
fn listed_files(stdout: &str) -> Vec<PathBuf> {
    stdout
        .lines()
        .map(str::trim_end)
        .filter(|l| Path::new(l).is_absolute() && !l.contains(" - ") && Path::new(l).is_file())
        .map(PathBuf::from)
        .collect()
}

/// Resolve the per-invocation tsgo timeout. `SVN_TSGO_TIMEOUT_SECS`
/// overrides the default; `0` (or a value that doesn't parse) disables
/// the cap. A disabled cap is represented as `Duration::MAX`, which the
/// poll loop never reaches.
fn tsgo_timeout() -> Duration {
    match std::env::var("SVN_TSGO_TIMEOUT_SECS") {
        Ok(v) => match v.trim().parse::<u64>() {
            Ok(0) => Duration::MAX,
            Ok(secs) => Duration::from_secs(secs),
            Err(_) => Duration::from_secs(DEFAULT_TSGO_TIMEOUT_SECS),
        },
        Err(_) => Duration::from_secs(DEFAULT_TSGO_TIMEOUT_SECS),
    }
}

/// Last few stderr lines, for the failure message. tsgo's panic/abort
/// output lands on stderr; the tail is the actionable part.
/// What the compiler printed, for the failure message: unparsed output
/// such as a global error or an unknown compiler option is often the
/// only explanation. The `--listFiles` paths are dropped, and the text
/// is capped at the 2000 characters svelte-check shows.
fn failure_output(stdout: &str, stderr: &str) -> String {
    let listed: std::collections::HashSet<PathBuf> = listed_files(stdout).into_iter().collect();
    let text: Vec<&str> = stdout
        .lines()
        .chain(stderr.lines())
        .filter(|l| !l.trim().is_empty() && !listed.contains(Path::new(l.trim())))
        .collect();
    text.join("\n").chars().take(2000).collect()
}

/// Result of draining a child to completion (or killing it on timeout).
struct Wait {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    timed_out: bool,
}

/// Wait for `child`, draining stdout/stderr on background threads so a
/// full pipe buffer can't deadlock us, and killing it if `timeout` is
/// exceeded. `Duration::MAX` disables the deadline.
///
/// Replaces a bare `Command::output()`, which both ignored the exit
/// status and could block forever on a hung tsgo.
fn wait_with_timeout(mut child: Child, timeout: Duration) -> std::io::Result<Wait> {
    // Drain the pipes concurrently — `output()`'s job, but we need the
    // child handle for try_wait/kill, so we do it by hand. Each reader
    // additionally signals pipe-EOF over a channel: process exit closes
    // the child's pipe ends, so EOF is the event that wakes the wait
    // loop below the moment tsgo finishes. A fixed-interval poll here
    // used to round every tsgo run up to the next 50ms boundary.
    let (eof_tx, eof_rx) = std::sync::mpsc::channel::<()>();
    let mut child_stdout = child.stdout.take();
    let mut child_stderr = child.stderr.take();
    let stdout_reader = {
        let eof_tx = eof_tx.clone();
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(s) = child_stdout.as_mut() {
                let _ = s.read_to_end(&mut buf);
            }
            let _ = eof_tx.send(());
            buf
        })
    };
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = child_stderr.as_mut() {
            let _ = s.read_to_end(&mut buf);
        }
        let _ = eof_tx.send(());
        buf
    });

    // Interval at which the deadline stays live even when the child
    // produces no pipe events (hung without closing its pipes). Only
    // bounds how LATE a timeout kill can fire, not how fast a normal
    // exit is observed — that's the EOF wakeup.
    const POLL_INTERVAL: Duration = Duration::from_millis(50);

    let deadline = Instant::now().checked_add(timeout);
    let mut timed_out = false;
    let mut pipes_eof = false;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            let _ = child.kill();
            timed_out = true;
            break child.wait()?;
        }
        if pipes_eof {
            // Both pipes hit EOF but the child isn't reapable yet.
            // Normally exit is imminent (EOF is a consequence of the
            // process tearing down), so this fine-grained poll runs
            // once or twice. A child that closed its pipes early and
            // kept running degrades to a 1ms poll bounded by the
            // deadline above.
            std::thread::sleep(Duration::from_millis(1));
        } else {
            // Sleep until a pipe reaches EOF or the poll interval
            // elapses, whichever comes first. Disconnection means
            // both reader threads finished (their senders dropped) —
            // every subsequent wakeup comes from the branch above.
            let interval = match deadline {
                Some(d) => d
                    .saturating_duration_since(Instant::now())
                    .min(POLL_INTERVAL),
                None => POLL_INTERVAL,
            };
            match eof_rx.recv_timeout(interval) {
                Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => pipes_eof = true,
            }
        }
    };

    let stdout = stdout_reader.join().unwrap_or_default();
    let stderr = stderr_reader.join().unwrap_or_default();
    Ok(Wait {
        status,
        stdout,
        stderr,
        timed_out,
    })
}

/// Pull the `--extendedDiagnostics` stats block out of tsgo's stdout.
/// The block is at the tail: a sequence of `<Label>: <value>` lines
/// starting with `Files:` and ending with `Total time:`. We scan from
/// the end backwards to find `Total time:`, then walk up to the first
/// `Files:` label.
fn extract_extended_diagnostics(stdout: &str) -> Option<String> {
    let lines: Vec<&str> = stdout.lines().collect();
    let total_idx = lines
        .iter()
        .rposition(|line| line.trim_start().starts_with("Total time:"))?;
    let files_idx = lines[..=total_idx]
        .iter()
        .rposition(|line| line.trim_start().starts_with("Files:"))?;
    Some(lines[files_idx..=total_idx].join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_trailing_stats_block() {
        let stdout = "\
src/foo.ts(1,1): error TS2304: Cannot find name 'foo'.
Files:                   6250
Lines:                 395027
Memory used:          369425K
Total time:            1.237s
";
        let got = extract_extended_diagnostics(stdout).unwrap();
        assert!(got.starts_with("Files:"));
        assert!(got.contains("Memory used:"));
        assert!(got.ends_with("Total time:            1.237s"));
    }

    #[test]
    fn absent_block_returns_none() {
        let stdout = "src/foo.ts(1,1): error TS2304: Cannot find name 'foo'.\n";
        assert!(extract_extended_diagnostics(stdout).is_none());
    }

    #[test]
    fn partial_block_returns_none() {
        // Total time present but Files: absent — shouldn't invent a block.
        let stdout = "Total time:            1.237s\n";
        assert!(extract_extended_diagnostics(stdout).is_none());
    }

    #[test]
    fn clean_exit_never_fails() {
        assert!(!run_failed(Some(0), false));
        assert!(!run_failed(Some(0), true));
    }

    #[test]
    fn nonzero_exit_fails_only_without_diagnostics() {
        // Exit 1 or 2 is how the engines say "found errors".
        assert!(!run_failed(Some(1), true));
        assert!(!run_failed(Some(2), true));
        // The same codes with nothing parsed: a global error, an unknown
        // option, or a crash reported as a code (as Windows reports kills).
        assert!(run_failed(Some(1), false));
        assert!(run_failed(Some(139), false));
    }

    #[test]
    fn signal_always_fails() {
        // A killed run is partial even if it printed diagnostics first.
        assert!(run_failed(None, false));
        assert!(run_failed(None, true));
    }

    #[test]
    fn failure_output_drops_listed_files_and_blanks() {
        let tmp = tempfile::tempdir().unwrap();
        let listed = tmp.path().join("a.ts");
        std::fs::write(&listed, "").unwrap();
        let stdout = format!(
            "\n{}\nerror TS5023: Unknown compiler option 'x'.\n",
            listed.display()
        );
        let got = failure_output(&stdout, "\nfatal: out of memory\n");
        assert_eq!(
            got,
            "error TS5023: Unknown compiler option 'x'.\nfatal: out of memory"
        );
    }
}
