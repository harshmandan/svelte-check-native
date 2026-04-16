//! Spawn tsgo against a populated cache and parse its output.
//!
//! The orchestrator (in `lib.rs`) populates the cache with generated
//! `.svelte.ts` files and writes the overlay tsconfig; the runner then
//! invokes tsgo and converts its stdout into a [`Vec<RawDiagnostic>`].
//!
//! Invocation:
//!
//! ```text
//! tsgo --project <overlay.json> --pretty true --noErrorTruncation
//! ```
//!
//! `--pretty true` and `--noErrorTruncation` mirror upstream svelte-check's
//! invocation; both make output deterministic for parsing.

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

/// Run tsgo against an overlay tsconfig. Returns the parsed diagnostics
/// (raw — paths still point at generated files; the orchestrator maps them
/// back to source).
pub fn run(tsgo: &TsgoBinary, overlay_tsconfig: &Path) -> Result<Vec<RawDiagnostic>, RunError> {
    let output = if tsgo.needs_node {
        Command::new("node")
            .arg(&tsgo.path)
            .arg("--project")
            .arg(overlay_tsconfig)
            .arg("--pretty")
            .arg("true")
            .arg("--noErrorTruncation")
            .output()
            .map_err(RunError::Spawn)?
    } else {
        Command::new(&tsgo.path)
            .arg("--project")
            .arg(overlay_tsconfig)
            .arg("--pretty")
            .arg("true")
            .arg("--noErrorTruncation")
            .output()
            .map_err(RunError::Spawn)?
    };

    let mut combined = String::new();
    combined.push_str(&String::from_utf8_lossy(&output.stdout));
    combined.push('\n');
    combined.push_str(&String::from_utf8_lossy(&output.stderr));

    Ok(parse_output(&combined))
}
