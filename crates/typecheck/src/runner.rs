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
use std::path::Path;
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
    /// tsgo exited abnormally (killed by a signal, or an exit code
    /// other than 0/1) AND produced no parseable diagnostics. We treat
    /// this as a hard failure rather than a clean 0-error run — a
    /// crashing/OOMing tsgo that we report as "clean" is the worst
    /// failure mode for a checker (false-clean in CI).
    #[error(
        "tsgo exited abnormally{} without producing diagnostics{}",
        match .code { Some(c) => format!(" (exit code {c})"), None => " (killed by signal)".into() },
        if .stderr.is_empty() { String::new() } else { format!(":\n{}", .stderr) }
    )]
    Failed { code: Option<i32>, stderr: String },
    /// tsgo did not finish within the configured timeout and was killed.
    #[error("tsgo timed out after {}s and was killed", .0.as_secs())]
    Timeout(Duration),
}

/// tsc/tsgo exit-code semantics (`ExitStatus`): `0` = success, `1` =
/// diagnostics reported with outputs skipped (the normal "found errors"
/// path under our forced `noEmit`), `2` = diagnostics reported with outputs
/// generated, `3`+ = invalid project / fatal. We force `noEmit` in the
/// overlay, so a diagnostics run exits `1`, never `2`; treating `2` as
/// abnormal is therefore safe (and the `diagnostics.is_empty()` guard at the
/// call site keeps any stray code-2 run with diagnostics on the `Ok` path).
/// `None` (death by signal) likewise means tsgo failed to complete.
fn exited_abnormally(code: Option<i32>) -> bool {
    !matches!(code, Some(0) | Some(1))
}

/// What `run` returns: the parsed diagnostics and an optional
/// extended-diagnostics block.
#[derive(Debug)]
pub struct RunOutput {
    pub diagnostics: Vec<RawDiagnostic>,
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

    // A non-zero/abnormal exit that yielded NO diagnostics is a tsgo
    // crash, not a clean run — surface it (caller maps RunError to
    // exit 2) instead of silently reporting 0 errors. We deliberately
    // keep diagnostics from a normal exit-1 ("found errors") run, and
    // also keep whatever partial diagnostics an abnormal exit managed
    // to print rather than discarding a near-complete stream.
    if exited_abnormally(status.code()) && diagnostics.is_empty() {
        let tail = stderr_tail(&stderr);
        return Err(RunError::Failed {
            code: status.code(),
            stderr: tail,
        });
    }

    Ok(RunOutput {
        diagnostics,
        extended_diagnostics: extended_diag_text,
    })
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
fn stderr_tail(stderr: &str) -> String {
    let lines: Vec<&str> = stderr.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(8);
    lines[start..].join("\n")
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
    // child handle for try_wait/kill, so we do it by hand.
    let mut child_stdout = child.stdout.take();
    let mut child_stderr = child.stderr.take();
    let stdout_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = child_stdout.as_mut() {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });
    let stderr_reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        if let Some(s) = child_stderr.as_mut() {
            let _ = s.read_to_end(&mut buf);
        }
        buf
    });

    let deadline = Instant::now().checked_add(timeout);
    let mut timed_out = false;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break status,
            None => {
                if let Some(deadline) = deadline
                    && Instant::now() >= deadline
                {
                    let _ = child.kill();
                    timed_out = true;
                    break child.wait()?;
                }
                std::thread::sleep(Duration::from_millis(50));
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
    fn exit_0_and_1_are_normal() {
        // 0 = clean, 1 = diagnostics reported. Neither is a crash.
        assert!(!exited_abnormally(Some(0)));
        assert!(!exited_abnormally(Some(1)));
    }

    #[test]
    fn other_codes_and_signals_are_abnormal() {
        // Code 2 (diagnostics + outputs generated) never happens under forced
        // noEmit, so we classify it as abnormal; code 3 (invalid project) and
        // death by signal (None) are genuine failures.
        assert!(exited_abnormally(Some(2)));
        assert!(exited_abnormally(Some(3)));
        assert!(exited_abnormally(Some(139))); // 128 + SIGSEGV
        assert!(exited_abnormally(None));
    }

    #[test]
    fn stderr_tail_keeps_last_lines_drops_blanks() {
        let stderr = "\n\nline a\nline b\n\nline c\n";
        let got = stderr_tail(stderr);
        assert_eq!(got, "line a\nline b\nline c");
    }
}
