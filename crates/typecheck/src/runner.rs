//! Spawn tsgo against a populated cache and parse its output.
//!
//! The orchestrator (in `lib.rs`) populates the cache with generated
//! `.svelte.ts` files and writes the overlay tsconfig; the runner then
//! invokes tsgo and converts its stdout into a [`RunOutput`] of
//! diagnostics + program file count.
//!
//! Invocation:
//!
//! ```text
//! tsgo --project <overlay.json> --pretty true --noErrorTruncation --listFiles [--extendedDiagnostics]
//! ```
//!
//! `--pretty true` and `--noErrorTruncation` mirror upstream svelte-check's
//! invocation; `--listFiles` makes tsgo print every file in its program
//! interspersed with the diagnostic stream, so we can count what tsgo
//! actually loaded (matches upstream svelte-check's `<N> FILES` denominator
//! in the COMPLETED line). `--extendedDiagnostics` is added when the user
//! passes `--tsgo-diagnostics`; its stats block (file/line/symbol counts,
//! memory use, phase timings) is captured and returned for the CLI to print.

use std::path::Path;
use std::process::Command;

use crate::discovery::TsgoBinary;
use crate::output::{RawDiagnostic, parse as parse_output};

/// Errors when running tsgo.
#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error("failed to spawn tsgo: {0}")]
    Spawn(#[source] std::io::Error),
}

/// What `run` returns: the parsed diagnostics plus the number of files
/// in tsgo's program (collected from `--listFiles`).
#[derive(Debug)]
pub struct RunOutput {
    pub diagnostics: Vec<RawDiagnostic>,
    /// Count of every file in tsgo's program — `.svelte.ts` overlays,
    /// user `.ts`/`.tsx`/etc., transitively imported `.d.ts` from
    /// `node_modules`, all `lib.*.d.ts` libs. Matches the denominator
    /// upstream svelte-check prints in its COMPLETED line.
    pub program_file_count: usize,
    /// `--extendedDiagnostics` block captured verbatim from tsgo's
    /// stdout tail. `Some(text)` iff the caller requested extended
    /// diagnostics AND tsgo emitted a recognizable block. Text is the
    /// trailing lines starting from the first `Files:` label and
    /// running through tsgo's final `Total time:` line.
    pub extended_diagnostics: Option<String>,
}

/// Run tsgo against an overlay tsconfig. Returns the parsed diagnostics
/// + program file count + optional extended-diagnostics block.
///
/// `workspace` is set as tsgo's working directory so the diagnostic paths
/// it emits (which are relative to its cwd) resolve under the workspace
/// root via `workspace.join()`. Without this, running the binary from a
/// monorepo root with `--workspace ./apps/admin` produces phantom paths
/// like `apps/admin/apps/admin/.svelte-check/tsconfig.json`, which
/// breaks the overlay-noise filter and leaks intentional config-flag
/// deprecations (e.g. TS5102 baseUrl) as user-visible errors.
///
/// When `extended_diagnostics` is true, `--extendedDiagnostics` is
/// appended to tsgo's argv; the stats block tsgo emits after the last
/// diagnostic is captured in the returned `extended_diagnostics` field.
pub fn run(
    tsgo: &TsgoBinary,
    overlay_tsconfig: &Path,
    workspace: &Path,
    extended_diagnostics: bool,
) -> Result<RunOutput, RunError> {
    let mut args: Vec<std::ffi::OsString> = vec![
        "--project".into(),
        overlay_tsconfig.into(),
        "--pretty".into(),
        "true".into(),
        "--noErrorTruncation".into(),
        "--listFiles".into(),
    ];
    if extended_diagnostics {
        args.push("--extendedDiagnostics".into());
    }

    let output = if tsgo.needs_node {
        let mut cmd = Command::new("node");
        cmd.arg(&tsgo.path);
        cmd.args(&args);
        cmd.current_dir(workspace);
        cmd.output().map_err(RunError::Spawn)?
    } else {
        let mut cmd = Command::new(&tsgo.path);
        cmd.args(&args);
        cmd.current_dir(workspace);
        cmd.output().map_err(RunError::Spawn)?
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // `--listFiles` writes one absolute path per program file to stdout.
    // Diagnostics print as `<path>(<line>,<col>): error TS<N>: <msg>` —
    // distinguishable because the file-list lines are bare paths with
    // no parens or "error TS" marker. Counting lines that start with a
    // path-separator and don't contain a diagnostic-shaped suffix keeps
    // it simple and robust against `--pretty true` ANSI escapes.
    let program_file_count = stdout
        .lines()
        .filter(|line| {
            let trimmed = line.trim_start_matches(|c: char| !c.is_alphanumeric() && c != '/');
            trimmed.starts_with('/')
                && !line.contains("): error TS")
                && !line.contains("): warning TS")
        })
        .count();

    let extended_diag_text = if extended_diagnostics {
        extract_extended_diagnostics(&stdout)
    } else {
        None
    };

    let mut combined = String::new();
    combined.push_str(&stdout);
    combined.push('\n');
    combined.push_str(&stderr);

    Ok(RunOutput {
        diagnostics: parse_output(&combined),
        program_file_count,
        extended_diagnostics: extended_diag_text,
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
}
