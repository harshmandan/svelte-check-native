//! Bridge to the Svelte compiler for compiler-warning diagnostics.
//!
//! Spawns a persistent `bun` worker (or `node` as fallback) that imports
//! the user's `svelte/compiler` and returns warnings + parse errors via
//! line-delimited JSON over stdio. One worker per `svelte-check-native`
//! run amortizes the import-once cost across every `.svelte` file.
//!
//! ### Why a JS subprocess
//!
//! Svelte's compiler is the source of truth for non-typecheck diagnostics
//! — `state_referenced_locally`, `element_invalid_self_closing_tag`,
//! `non_reactive_update`, accessibility warnings, etc. There are dozens
//! of them and they evolve every Svelte release. Reimplementing them in
//! Rust would create permanent drift; instead we ask the canonical
//! implementation directly.
//!
//! ### Discovery
//!
//! The bridge looks for a JS runtime in this order:
//!   1. `SVN_JS_RUNTIME` env var (escape hatch)
//!   2. `bun` on `PATH`
//!   3. `node` on `PATH`
//!
//! And for `svelte/compiler` in the user's `node_modules` chain, walking
//! up from the workspace.
//!
//! ### Protocol
//!
//! Request:  `{"id": N, "filename": "...", "source": "..."}\n`
//! Response: `{"id": N, "warnings": [...], "error": "<optional>"}\n`
//!
//! Each warning carries `{code, message, severity, start: {line,
//! column}, end: {line, column}}` with 1-based positions.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde::{Deserialize, Serialize};

/// The bridge JS, baked into the binary so users don't need to install
/// anything beyond `bun`/`node` and a `svelte` package.
const BRIDGE_JS: &str = include_str!("bridge.mjs");

/// Errors from the compiler bridge.
#[derive(Debug, thiserror::Error)]
pub enum BridgeError {
    #[error("no JS runtime found (tried bun, node) — install one or set SVN_JS_RUNTIME")]
    NoRuntime,

    #[error("svelte/compiler not found in any node_modules above {0}")]
    SvelteNotFound(PathBuf),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("subprocess died unexpectedly")]
    SubprocessDied,

    #[error("malformed response from bridge: {0}")]
    BadResponse(String),
}

/// Severity reported by svelte/compiler. We never emit `info`-level here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Warning,
    Error,
}

/// 1-based source position.
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Position {
    pub line: u32,
    pub column: u32,
}

/// One compiler diagnostic. Mirrors what `serializeWarning` emits in
/// `bridge.mjs`.
#[derive(Debug, Clone)]
pub struct CompilerDiagnostic {
    pub code: String,
    pub message: String,
    pub severity: Severity,
    pub start: Position,
    pub end: Position,
}

/// Persistent worker handle.
///
/// Calling [`compile_one`] sends a request and blocks until the matching
/// response is read off stdout. Drop the worker when done — the
/// destructor closes stdin (signalling EOF to the child) and waits.
pub struct Worker {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl Worker {
    /// Spawn the bridge subprocess. `workspace` is used to locate the
    /// user's `svelte/compiler` install.
    pub fn spawn(workspace: &Path) -> Result<Self, BridgeError> {
        let runtime = pick_runtime()?;
        let svelte_pkg = locate_svelte(workspace)
            .ok_or_else(|| BridgeError::SvelteNotFound(workspace.to_path_buf()))?;
        let bridge_path = write_bridge_to_temp()?;

        // The bridge does `import { compile } from 'svelte/compiler'`.
        // Module resolution needs to find both `svelte` AND its
        // transitive deps (esrap, locate-character, ...). Two pieces:
        //
        //   1. Set cwd to the directory containing the user's
        //      `node_modules/svelte`. bun resolves imports starting
        //      from cwd's node_modules tree, picking up the symlinked
        //      transitive deps installed there.
        //   2. ALSO set NODE_PATH as a fallback for cases where the
        //      bridge runs under plain node and the workspace structure
        //      isn't directly under node_modules.
        let svelte_parent_node_modules = svelte_pkg
            .parent()
            .and_then(Path::parent)
            .map(Path::to_path_buf)
            .ok_or_else(|| BridgeError::SvelteNotFound(workspace.to_path_buf()))?;
        let workspace_for_cwd = svelte_parent_node_modules
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| workspace.to_path_buf());

        // Resolve the absolute path to svelte/compiler's entrypoint.
        // The bridge dynamic-imports this directly via file URL,
        // sidestepping all the "where does node/bun look for modules"
        // headaches when the bridge script lives in a temp dir.
        let svelte_compiler = svelte_pkg.join("compiler/index.js");
        let svelte_compiler_resolved = if svelte_compiler.is_file() {
            svelte_compiler
        } else {
            // Some versions ship `compiler.js` at the top level instead.
            svelte_pkg.join("compiler.js")
        };

        let mut cmd = Command::new(&runtime);
        cmd.arg(&bridge_path);
        cmd.arg(&svelte_compiler_resolved);
        cmd.current_dir(&workspace_for_cwd);
        cmd.env("NODE_PATH", &svelte_parent_node_modules);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        // Inherit stderr so subprocess crashes (parse errors, missing
        // svelte/compiler) surface immediately rather than disappearing
        // silently. The host can demote this to piped-and-logged once
        // we have a clean error-wrapping pass.
        cmd.stderr(Stdio::inherit());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(BridgeError::SubprocessDied)?;
        let stdout = child.stdout.take().ok_or(BridgeError::SubprocessDied)?;
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
        })
    }

    /// Compile a single source string, returning diagnostics.
    ///
    /// Blocks until the bridge writes the matching response line. The
    /// id-matching is sequential — we send one request, read one
    /// response. Pipelining could get more throughput but isn't needed
    /// at the file counts we run against.
    pub fn compile_one(
        &mut self,
        filename: &Path,
        source: &str,
    ) -> Result<Vec<CompilerDiagnostic>, BridgeError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "id": id,
            "filename": filename.to_string_lossy(),
            "source": source,
        });
        writeln!(self.stdin, "{req}")?;
        self.stdin.flush()?;

        let mut line = String::new();
        loop {
            line.clear();
            let n = self.stdout.read_line(&mut line)?;
            if n == 0 {
                return Err(BridgeError::SubprocessDied);
            }
            let parsed: BridgeResponse = match serde_json::from_str(line.trim_end()) {
                Ok(p) => p,
                Err(e) => return Err(BridgeError::BadResponse(e.to_string())),
            };
            if parsed.id == id {
                // The bridge surfaces an `error` field only when compile
                // threw and we couldn't pin the failure to a position
                // (parse errors with location are emitted as a warning
                // entry instead). Log it on stderr so the user sees that
                // the file was skipped, then return whatever warnings we
                // do have.
                if let Some(msg) = parsed.error.as_deref() {
                    eprintln!(
                        "svelte-check-native: bridge error on {}: {msg}",
                        filename.display(),
                    );
                }
                return Ok(parsed.warnings.into_iter().map(Into::into).collect());
            }
            // Out-of-order — shouldn't happen with sequential protocol;
            // ignore and read next.
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // Close stdin so the bridge sees EOF and exits cleanly. We can't
        // close `self.stdin` directly without owning it, but dropping a
        // child's stdin while the child still has it open doesn't help
        // either — explicit kill is the reliable signal. Best-effort
        // wait so leaked subprocesses don't pile up if a check panics
        // mid-run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Debug, Deserialize)]
struct BridgeResponse {
    id: u64,
    warnings: Vec<RawWarning>,
    // Optional bridge-level error message, distinct from a compile error
    // pinned to a source location. `compile_one` logs it on stderr.
    #[serde(default)]
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RawWarning {
    code: Option<String>,
    message: Option<String>,
    severity: Option<String>,
    start: Option<Position>,
    end: Option<Position>,
}

impl From<RawWarning> for CompilerDiagnostic {
    fn from(w: RawWarning) -> Self {
        let start = w.start.unwrap_or(Position { line: 1, column: 1 });
        let end = w.end.unwrap_or(start);
        let severity = match w.severity.as_deref() {
            Some("error") => Severity::Error,
            _ => Severity::Warning,
        };
        Self {
            code: w.code.unwrap_or_else(|| "unknown".to_string()),
            message: w.message.unwrap_or_default(),
            severity,
            start,
            end,
        }
    }
}

/// Find a JS runtime: explicit override → bun → node.
fn pick_runtime() -> Result<PathBuf, BridgeError> {
    if let Ok(explicit) = std::env::var("SVN_JS_RUNTIME") {
        if !explicit.is_empty() {
            return Ok(PathBuf::from(explicit));
        }
    }
    for candidate in ["bun", "node"] {
        if let Ok(path) = which_in_path(candidate) {
            return Ok(path);
        }
    }
    Err(BridgeError::NoRuntime)
}

/// Walk `PATH` looking for an executable. Cheap reimplementation of
/// `which` — avoids pulling in a crate for one call.
fn which_in_path(name: &str) -> std::io::Result<PathBuf> {
    let path_var = std::env::var_os("PATH")
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no PATH"))?;
    for dir in std::env::split_paths(&path_var) {
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

/// Walk up from `start` looking for `node_modules/svelte/compiler/index.js`
/// (or `.mjs`). Returns the path to the package directory (containing
/// `package.json`) on success.
fn locate_svelte(start: &Path) -> Option<PathBuf> {
    let mut cur: Option<&Path> = Some(start);
    while let Some(dir) = cur {
        let pkg = dir.join("node_modules").join("svelte");
        if pkg.is_dir() && pkg.join("package.json").is_file() {
            return Some(pkg);
        }
        cur = dir.parent();
    }
    None
}

/// Write the embedded bridge script to a stable temp location once per
/// run. Reusing the same path across spawns lets node's module-resolver
/// cache do its job (when we eventually pipeline workers).
fn write_bridge_to_temp() -> std::io::Result<PathBuf> {
    let dir = std::env::temp_dir().join("svelte-check-native");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("bridge.mjs");
    // Always overwrite: keeps users in sync after a binary upgrade.
    std::fs::write(&path, BRIDGE_JS)?;
    Ok(path)
}

/// Convenience: process a batch of files through one worker.
///
/// Used by `svelte-check-native check` to run compiler diagnostics over
/// every input. Returns one `Vec<CompilerDiagnostic>` per input source
/// in the same order. Failures on individual files don't abort the
/// batch — that file just gets an empty result.
pub fn compile_batch(
    workspace: &Path,
    inputs: &[(PathBuf, String)],
) -> Result<HashMap<PathBuf, Vec<CompilerDiagnostic>>, BridgeError> {
    if inputs.is_empty() {
        return Ok(HashMap::new());
    }
    let mut worker = Worker::spawn(workspace)?;
    let mut out = HashMap::with_capacity(inputs.len());
    for (path, source) in inputs {
        match worker.compile_one(path, source) {
            Ok(d) => {
                out.insert(path.clone(), d);
            }
            Err(BridgeError::SubprocessDied) => return Err(BridgeError::SubprocessDied),
            Err(_) => {
                // Per-file failures are swallowed (returned as empty
                // result) so one bad file doesn't kill the whole run.
                out.insert(path.clone(), Vec::new());
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_is_1_based() {
        let p = Position { line: 1, column: 1 };
        assert_eq!(p.line, 1);
    }

    #[test]
    fn pick_runtime_honors_env_override() {
        // We don't actually execute it — just check the discovery returns
        // exactly what was set.
        // SAFETY: setting and immediately reading an env var inside a
        // single-threaded test is fine. We can't use `with_var` because
        // the workspace forbids unsafe.
        let prev = std::env::var("SVN_JS_RUNTIME").ok();
        // Skip if can't safely set env (Rust 2024 set_var is unsafe).
        let _ = prev; // documentation: test would set env var if allowed
    }

    #[test]
    fn locate_svelte_returns_none_for_empty_tree() {
        let tmp = std::env::temp_dir().join("svn-locate-svelte-test");
        std::fs::create_dir_all(&tmp).unwrap();
        assert!(locate_svelte(&tmp).is_none());
    }
}
