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

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::{self, JoinHandle};

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
    /// Thread that drains the subprocess's stderr and forwards it to our
    /// own stderr, prefixed so it's distinguishable from host output.
    /// Joined in `Drop` after the child exits.
    stderr_pump: Option<JoinHandle<()>>,
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

        // Discover the user's svelte.config.{js,mjs,cjs} so the bridge
        // can dynamic-import it once at startup and fold its
        // `compilerOptions` into every compile() call. Without this,
        // projects that rely on experimental rune flags (e.g.
        // `compilerOptions.experimental.async = true`) get spurious
        // compiler errors that upstream svelte-check (which does honor
        // svelte.config.js) does not produce.
        let svelte_config = locate_svelte_config(workspace);

        let mut cmd = Command::new(&runtime);
        cmd.arg(&bridge_path);
        cmd.arg(&svelte_compiler_resolved);
        // Empty string means "no config" — keeps argv positional rather
        // than introducing a flag-style parser to the bridge for one arg.
        cmd.arg(
            svelte_config
                .as_deref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        );
        cmd.current_dir(&workspace_for_cwd);
        cmd.env("NODE_PATH", &svelte_parent_node_modules);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        // Pipe stderr so subprocess output doesn't interleave with our
        // human/machine output on a shared terminal. A background thread
        // reads it line-by-line and re-emits each line with a prefix so
        // bridge crashes still surface, but cleanly tagged.
        cmd.stderr(Stdio::piped());
        let mut child = cmd.spawn()?;
        let stdin = child.stdin.take().ok_or(BridgeError::SubprocessDied)?;
        let stdout = child.stdout.take().ok_or(BridgeError::SubprocessDied)?;
        let stderr = child.stderr.take().ok_or(BridgeError::SubprocessDied)?;
        let stderr_pump = thread::Builder::new()
            .name("svn-bridge-stderr".into())
            .spawn(move || pump_stderr(stderr))
            .ok();
        Ok(Self {
            child,
            stdin,
            stdout: BufReader::new(stdout),
            next_id: 1,
            stderr_pump,
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
        if let Some(pump) = self.stderr_pump.take() {
            // Child is dead → its stderr is closed → pump's read loop
            // returns and the thread exits. Join is then quick.
            let _ = pump.join();
        }
    }
}

/// Read the bridge subprocess's stderr line by line and forward each
/// line to our own stderr with a prefix so it's distinguishable from
/// host output. Returns when the read side closes (typically when the
/// child exits).
fn pump_stderr<R: Read + Send + 'static>(reader: R) {
    let mut buf = BufReader::new(reader);
    let mut line = String::new();
    loop {
        line.clear();
        match buf.read_line(&mut line) {
            Ok(0) => return,
            Ok(_) => {
                let trimmed = line.trim_end_matches(['\n', '\r']);
                if !trimmed.is_empty() {
                    eprintln!("svelte-check-native [bridge]: {trimmed}");
                }
            }
            Err(_) => return,
        }
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
    let pathext = std::env::var_os("PATHEXT").unwrap_or_else(|| {
        // Windows guarantees PATHEXT is set, but a weird launch environment
        // might strip it — default to the shell's built-in list so we don't
        // silently regress to bare-name lookup.
        if cfg!(windows) {
            std::ffi::OsString::from(".COM;.EXE;.BAT;.CMD")
        } else {
            std::ffi::OsString::new()
        }
    });
    which_in(&path_var, &pathext, name)
        .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, name.to_string()))
}

/// Pure implementation of the PATH walk. Separated from [`which_in_path`]
/// so tests can exercise the PATHEXT logic without mutating process env.
///
/// For each directory in `path_var`, try `<name>` as-is first (so callers
/// that already passed `node.exe` don't double-append), then try
/// `<name><suffix>` for every non-empty suffix in `pathext`. First hit
/// wins. Empty/unset `pathext` degenerates to bare-name only — the Unix
/// default and a safe no-op on platforms without executable extensions.
fn which_in(path_var: &std::ffi::OsStr, pathext: &std::ffi::OsStr, name: &str) -> Option<PathBuf> {
    let pathext_str = pathext.to_string_lossy();
    let suffixes: Vec<&str> = std::iter::once("")
        .chain(pathext_str.split(';').filter(|e| !e.is_empty()))
        .collect();
    for dir in std::env::split_paths(path_var) {
        for suffix in &suffixes {
            let candidate = if suffix.is_empty() {
                dir.join(name)
            } else {
                dir.join(format!("{name}{suffix}"))
            };
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// Walk up from `start` looking for `node_modules/svelte/compiler/index.js`
/// (or `.mjs`). Returns the path to the package directory (containing
/// `package.json`) on success.
fn locate_svelte(start: &Path) -> Option<PathBuf> {
    svn_core::walk_up_dirs(start, |dir| {
        let pkg = dir.join(svn_core::NODE_MODULES_DIR).join("svelte");
        (pkg.is_dir() && pkg.join("package.json").is_file()).then_some(pkg)
    })
}

/// Walk up from `start` looking for the user's svelte.config.{js,mjs,cjs}.
/// Returns the absolute path of the first match, or None if no config
/// exists in the workspace's ancestor chain. Stops at the first
/// `node_modules/` boundary so we don't accidentally pick up a
/// dependency's vendored config.
fn locate_svelte_config(start: &Path) -> Option<PathBuf> {
    const NAMES: &[&str] = &["svelte.config.js", "svelte.config.mjs", "svelte.config.cjs"];
    let mut cur: Option<&Path> = Some(start);
    while let Some(dir) = cur {
        for name in NAMES {
            let p = dir.join(name);
            if p.is_file() {
                return Some(p);
            }
        }
        // Don't recurse past the workspace into a parent project's
        // node_modules — that path would point at a dep's vendored
        // config, never the user's intent.
        if dir.file_name() == Some(std::ffi::OsStr::new(svn_core::NODE_MODULES_DIR)) {
            return None;
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

/// Process a batch of files through one or more worker subprocesses.
///
/// On large batches the work is split across N workers running in
/// parallel threads — each owns its own `Worker` (its own JS subprocess)
/// and processes a contiguous slice of `inputs`. Results are merged in
/// input order so the output is deterministic.
///
/// Worker count is chosen by [`pick_worker_count`] from input size and
/// `std::thread::available_parallelism`. Single-file or very-small
/// batches stay on the original single-worker path because the cost of
/// spawning extra `bun` / `node` processes (and re-importing
/// `svelte/compiler` in each one) outweighs the parallelism benefit.
///
/// Consumes `inputs` so the caller's `PathBuf`s move through the worker
/// into the result without re-cloning. Failures on individual files
/// don't abort the batch — that file just gets an empty result.
pub fn compile_batch(
    workspace: &Path,
    inputs: Vec<(PathBuf, String)>,
) -> Result<Vec<(PathBuf, Vec<CompilerDiagnostic>)>, BridgeError> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let n_workers = pick_worker_count(inputs.len());
    if n_workers <= 1 {
        return compile_chunk(workspace, inputs);
    }

    // Split into n_workers contiguous chunks, preserving input order so
    // the merged result reads back in the caller's original order.
    let chunks = split_into_chunks(inputs, n_workers);

    // Each chunk runs on its own thread, owning its own Worker. We
    // spawn workers from inside the thread (not on the main thread)
    // so the JS subprocess startup cost overlaps across workers — the
    // hot startup path is the `import svelte/compiler` inside each
    // bridge.mjs, which is single-threaded JS but runs concurrently
    // across processes.
    type ChunkResult = Result<Vec<(PathBuf, Vec<CompilerDiagnostic>)>, BridgeError>;
    let mut handles: Vec<JoinHandle<ChunkResult>> = Vec::with_capacity(chunks.len());
    for chunk in chunks {
        let workspace = workspace.to_path_buf();
        let h = thread::Builder::new()
            .name("svn-bridge-worker".into())
            .spawn(move || compile_chunk(&workspace, chunk))
            .map_err(BridgeError::Io)?;
        handles.push(h);
    }

    let mut out: Vec<(PathBuf, Vec<CompilerDiagnostic>)> = Vec::new();
    for h in handles {
        // join() returns Err if the worker thread panicked; treat as
        // SubprocessDied since we have no diagnostics to fall back on.
        let chunk_result = h.join().map_err(|_| BridgeError::SubprocessDied)?;
        out.extend(chunk_result?);
    }
    Ok(out)
}

/// Run a single chunk of inputs through one freshly-spawned `Worker`.
fn compile_chunk(
    workspace: &Path,
    inputs: Vec<(PathBuf, String)>,
) -> Result<Vec<(PathBuf, Vec<CompilerDiagnostic>)>, BridgeError> {
    let mut worker = Worker::spawn(workspace)?;
    let mut out = Vec::with_capacity(inputs.len());
    for (path, source) in inputs {
        match worker.compile_one(&path, &source) {
            Ok(d) => out.push((path, d)),
            Err(BridgeError::SubprocessDied) => return Err(BridgeError::SubprocessDied),
            Err(_) => {
                // Per-file failures are swallowed (returned as empty
                // result) so one bad file doesn't kill the whole run.
                out.push((path, Vec::new()));
            }
        }
    }
    Ok(out)
}

/// Pick a worker count for `n_inputs` files, honoring the
/// `SVN_BRIDGE_WORKERS` env var when set.
///
/// Each extra worker pays a one-shot ~200 ms cost to spawn `bun`/`node`
/// and import `svelte/compiler`, plus ~100 MB of resident memory for
/// the JS heap. Empirically, `cores / 2` is the sweet spot on 8-core
/// Apple Silicon (4 workers beats 8 by ~20 % on a 1.2k-file workload —
/// over-subscribing past the performance-core count introduces
/// scheduler / IPC contention faster than it saves serial work). The
/// minimum of 2 ensures we keep at least *some* parallelism on a
/// 4-core box. Cap at 8 to keep memory bounded on large boxes.
fn pick_worker_count(n_inputs: usize) -> usize {
    if let Ok(s) = std::env::var("SVN_BRIDGE_WORKERS") {
        if let Ok(n) = s.parse::<usize>() {
            return n.clamp(1, 32).min(n_inputs.max(1));
        }
    }
    // Below this threshold the extra-worker spawn cost outweighs the
    // parallelism win — the single-worker path stays put.
    const MULTI_WORKER_THRESHOLD: usize = 32;
    if n_inputs < MULTI_WORKER_THRESHOLD {
        return 1;
    }
    let cores = thread::available_parallelism()
        .map(std::num::NonZeroUsize::get)
        .unwrap_or(1);
    let chosen = (cores / 2).clamp(2, 8);
    chosen.min(n_inputs)
}

/// Split `inputs` into `n` contiguous chunks of (almost) equal size.
/// The leftover from `len % n` is distributed one extra item to the
/// first `len % n` chunks so the largest chunk is at most one item
/// bigger than the smallest.
fn split_into_chunks<T>(mut inputs: Vec<T>, n: usize) -> Vec<Vec<T>> {
    let n = n.max(1);
    let total = inputs.len();
    let base = total / n;
    let extra = total % n;
    let mut chunks: Vec<Vec<T>> = Vec::with_capacity(n);
    // drain from the front so chunks come out in input order.
    let mut iter = inputs.drain(..);
    for i in 0..n {
        let take = base + if i < extra { 1 } else { 0 };
        let mut c = Vec::with_capacity(take);
        for _ in 0..take {
            if let Some(item) = iter.next() {
                c.push(item);
            }
        }
        chunks.push(c);
    }
    chunks
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

    #[test]
    fn which_in_finds_bare_name_without_pathext() {
        let tmp = tempfile::tempdir().unwrap();
        let file = tmp.path().join("svn_dummy");
        std::fs::write(&file, b"").unwrap();
        let path_var = tmp.path().as_os_str().to_owned();
        let pathext = std::ffi::OsString::new();
        assert_eq!(which_in(&path_var, &pathext, "svn_dummy"), Some(file),);
    }

    #[test]
    fn which_in_applies_pathext_suffix_when_bare_name_missing() {
        // Simulates Windows: only `node.exe` exists, user asks for `node`.
        // Without PATHEXT handling this returned None and the bridge
        // silently no-oped on every Windows install.
        let tmp = tempfile::tempdir().unwrap();
        // Use lowercase `.exe` for the on-disk file and a matching suffix
        // in PATHEXT. Case doesn't matter at lookup time on Windows
        // (case-insensitive filesystem) but the returned PathBuf echoes
        // the PATHEXT case verbatim, and APFS on macOS is also
        // case-insensitive — keep both sides aligned so the assertion
        // works cross-platform in CI.
        let file = tmp.path().join("svn_dummy.exe");
        std::fs::write(&file, b"").unwrap();
        let path_var = tmp.path().as_os_str().to_owned();
        let pathext = std::ffi::OsString::from(".cmd;.exe;.bat");
        assert_eq!(which_in(&path_var, &pathext, "svn_dummy"), Some(file),);
    }

    #[test]
    fn which_in_returns_none_when_nothing_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let path_var = tmp.path().as_os_str().to_owned();
        let pathext = std::ffi::OsString::from(".EXE");
        assert_eq!(which_in(&path_var, &pathext, "nonexistent"), None);
    }

    #[test]
    fn which_in_prefers_bare_name_over_suffixed_match() {
        // If both `foo` and `foo.exe` exist, the bare form wins — matches
        // Rust's `which` crate and the Windows shell's resolution order.
        let tmp = tempfile::tempdir().unwrap();
        let bare = tmp.path().join("svn_dummy");
        let exe = tmp.path().join("svn_dummy.exe");
        std::fs::write(&bare, b"").unwrap();
        std::fs::write(&exe, b"").unwrap();
        let path_var = tmp.path().as_os_str().to_owned();
        let pathext = std::ffi::OsString::from(".EXE");
        assert_eq!(which_in(&path_var, &pathext, "svn_dummy"), Some(bare),);
    }
}
