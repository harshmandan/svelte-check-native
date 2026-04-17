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
//! tsgo --project <overlay.json> --pretty true --noErrorTruncation --listFiles
//! ```
//!
//! `--pretty true` and `--noErrorTruncation` mirror upstream svelte-check's
//! invocation; `--listFiles` makes tsgo print every file in its program
//! interspersed with the diagnostic stream, so we can count what tsgo
//! actually loaded (matches upstream svelte-check's `<N> FILES` denominator
//! in the COMPLETED line).

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
}

/// Run tsgo against an overlay tsconfig. Returns the parsed diagnostics
/// + program file count.
///
/// `workspace` is set as tsgo's working directory so the diagnostic paths
/// it emits (which are relative to its cwd) resolve under the workspace
/// root via `workspace.join()`. Without this, running the binary from a
/// monorepo root with `--workspace ./apps/admin` produces phantom paths
/// like `apps/admin/apps/admin/.svelte-check/tsconfig.json`, which
/// breaks the overlay-noise filter and leaks intentional config-flag
/// deprecations (e.g. TS5102 baseUrl) as user-visible errors.
pub fn run(
    tsgo: &TsgoBinary,
    overlay_tsconfig: &Path,
    workspace: &Path,
) -> Result<RunOutput, RunError> {
    let output = if tsgo.needs_node {
        Command::new("node")
            .arg(&tsgo.path)
            .arg("--project")
            .arg(overlay_tsconfig)
            .arg("--pretty")
            .arg("true")
            .arg("--noErrorTruncation")
            .arg("--listFiles")
            .current_dir(workspace)
            .output()
            .map_err(RunError::Spawn)?
    } else {
        Command::new(&tsgo.path)
            .arg("--project")
            .arg(overlay_tsconfig)
            .arg("--pretty")
            .arg("true")
            .arg("--noErrorTruncation")
            .arg("--listFiles")
            .current_dir(workspace)
            .output()
            .map_err(RunError::Spawn)?
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

    let mut combined = String::new();
    combined.push_str(&stdout);
    combined.push('\n');
    combined.push_str(&stderr);

    Ok(RunOutput {
        diagnostics: parse_output(&combined),
        program_file_count,
    })
}
