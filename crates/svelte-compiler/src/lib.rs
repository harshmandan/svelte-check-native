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
//! And for `svelte/compiler` the way svelte-check loads it for each
//! component: node's resolution upwards from the component's own
//! directory, then from where this program is installed (svelte-check
//! falls back to its own install, which carries `svelte` as a peer).
//! Components that resolve different `svelte` installs are compiled by
//! different workers.
//!
//! ### Protocol
//!
//! Request:  `{"id": N, "filename": "...", "source": "..."}\n`
//! Response: `{"id": N, "warnings": [...], "error": "<optional>"}\n`
//!
//! Each warning carries `{code, message, severity, start: {line,
//! column}, end: {line, column}}` where line is 1-based, column is
//! 0-based (matching svelte/compiler's locate-character output).

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::thread::{self, JoinHandle};

use serde::{Deserialize, Serialize};

/// The bridge JS, baked into the binary so users don't need to install
/// anything beyond `bun`/`node` and a `svelte` package.
const BRIDGE_JS: &str = include_str!("bridge.mjs");
const TRANSPILE_JS: &str = include_str!("transpile.mjs");

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
    /// The compiler did not raise this: it broke with a JavaScript
    /// exception, which has no code (`code` is then a placeholder).
    pub exception: bool,
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
    /// The private directory holding the bridge script, removed once
    /// the worker is gone.
    _script_dir: tempfile::TempDir,
}

impl Worker {
    /// Spawn the bridge subprocess on the `svelte` package `svelte_pkg`
    /// (the directory holding its `package.json`). `workspace` is where
    /// the Svelte config is looked for.
    fn spawn_on(svelte_pkg: &Path, workspace: &Path, runtime: &Path) -> Result<Self, BridgeError> {
        let (script_dir, bridge_path) = write_script_to_temp("bridge", BRIDGE_JS)?;

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
            .ok_or_else(|| BridgeError::SvelteNotFound(svelte_pkg.to_path_buf()))?;
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

        let mut cmd = Command::new(runtime);
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
            _script_dir: script_dir,
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
        writeln!(self.stdin, "{}", request_line(&req))?;
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

    /// What svelte-check's default engine reports for a component
    /// svelte2tsx can't convert (see [`Rejection`]).
    pub fn rejection(&mut self, filename: &Path, source: &str) -> Result<Rejection, BridgeError> {
        let id = self.next_id;
        self.next_id += 1;
        let req = serde_json::json!({
            "id": id,
            "filename": filename.to_string_lossy(),
            "source": source,
            "rejection": true,
        });
        writeln!(self.stdin, "{}", request_line(&req))?;
        self.stdin.flush()?;
        let mut line = String::new();
        loop {
            line.clear();
            if self.stdout.read_line(&mut line)? == 0 {
                return Err(BridgeError::SubprocessDied);
            }
            let parsed: RejectionResponse = serde_json::from_str(line.trim_end())
                .map_err(|e| BridgeError::BadResponse(e.to_string()))?;
            if parsed.id == id {
                return Ok(Rejection {
                    compile_error: parsed.compile_error.map(Into::into),
                    parse_error: parsed.parse_error.map(Into::into),
                });
            }
        }
    }
}

/// What svelte-check's default engine reports for a component svelte2tsx
/// can't convert. Each is `None` when that step doesn't throw; an error
/// without a position is placed at the start of the file.
#[derive(Debug)]
pub struct Rejection {
    /// The compiler's error, compiled with the Svelte config's
    /// `compilerOptions` exactly as given (its Svelte pass).
    pub compile_error: Option<CompilerDiagnostic>,
    /// svelte2tsx's error: the compiler's `parse()` on the source with
    /// script and style contents blanked out (its TypeScript pass).
    pub parse_error: Option<CompilerDiagnostic>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RejectionResponse {
    id: u64,
    compile_error: Option<RawWarning>,
    parse_error: Option<RawWarning>,
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
/// One request as a line the bridge's `readline` reads whole.
/// `readline` also ends a line at U+2028 and U+2029, which JSON allows
/// raw inside strings: a component holding one would arrive as two
/// malformed halves and leave every later reply out of step.
fn request_line(request: &serde_json::Value) -> String {
    let line = request.to_string();
    if line.contains(['\u{2028}', '\u{2029}']) {
        line.replace('\u{2028}', "\\u2028")
            .replace('\u{2029}', "\\u2029")
    } else {
        line
    }
}

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
        let start = w.start.unwrap_or(Position { line: 1, column: 0 });
        let end = w.end.unwrap_or(start);
        let severity = match w.severity.as_deref() {
            Some("error") => Severity::Error,
            _ => Severity::Warning,
        };
        Self {
            exception: w.code.is_none(),
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

/// Walk up from `start` looking for `node_modules/svelte/package.json`,
/// as node resolves a package from a directory. Returns the package
/// directory on success.
fn locate_svelte(start: &Path) -> Option<PathBuf> {
    svn_core::walk_up_dirs(start, |dir| {
        let pkg = dir.join(svn_core::NODE_MODULES_DIR).join("svelte");
        (pkg.is_dir() && pkg.join("package.json").is_file()).then_some(pkg)
    })
}

/// The `svelte` package svelte-check would load the compiler of `file`
/// from: resolved from the file's directory, else from svelte-check's
/// own install. Ours is wherever this executable lives (by its real
/// path, as node gives a module's directory): a package manager places
/// a peer like `svelte` in a `node_modules` above it.
fn locate_svelte_for(file: &Path) -> Option<PathBuf> {
    file.parent().and_then(locate_svelte).or_else(|| {
        let exe = std::env::current_exe().ok()?;
        let exe = std::fs::canonicalize(&exe).unwrap_or(exe);
        locate_svelte(exe.parent()?)
    })
}

/// The `svelte` package for each of `files` (see [`locate_svelte_for`]),
/// with the indexes of the files that resolve to it, in first-seen
/// order. Fails when a file resolves none.
fn group_by_svelte<'a>(
    files: impl Iterator<Item = &'a Path>,
) -> Result<Vec<(PathBuf, Vec<usize>)>, BridgeError> {
    let mut by_dir: std::collections::HashMap<&Path, PathBuf> = std::collections::HashMap::new();
    let mut groups: Vec<(PathBuf, Vec<usize>)> = Vec::new();
    for (i, file) in files.enumerate() {
        let dir = file.parent().unwrap_or(file);
        let pkg = match by_dir.get(dir) {
            Some(pkg) => pkg.clone(),
            None => {
                let pkg = locate_svelte_for(file)
                    .ok_or_else(|| BridgeError::SvelteNotFound(dir.to_path_buf()))?;
                by_dir.insert(dir, pkg.clone());
                pkg
            }
        };
        match groups.iter_mut().find(|(p, _)| *p == pkg) {
            Some((_, members)) => members.push(i),
            None => groups.push((pkg, vec![i])),
        }
    }
    Ok(groups)
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

/// Write an embedded script to `<name>.mjs` in a new directory of its
/// own under the temp dir, returned with the script's path; the
/// directory is removed when it is dropped.
///
/// The directory is created with a random name and readable by this
/// user only, so nobody else can own it, swap the script between its
/// writing and the runtime loading it, or keep it from being written.
/// A fixed path in a temp dir shared between users (`/tmp` on Linux)
/// allows all three.
fn write_script_to_temp(name: &str, text: &str) -> std::io::Result<(tempfile::TempDir, PathBuf)> {
    let mut builder = tempfile::Builder::new();
    builder.prefix("svelte-check-native-");
    // tempfile leaves a directory's mode to the umask.
    #[cfg(unix)]
    builder.permissions(std::os::unix::fs::PermissionsExt::from_mode(0o700));
    let dir = builder.tempdir()?;
    let path = dir.path().join(format!("{name}.mjs"));
    std::fs::write(&path, text)?;
    Ok((dir, path))
}

/// Process a batch of files through one or more worker subprocesses.
///
/// Files are compiled by the `svelte` their own directory resolves (see
/// [`locate_svelte_for`]). On large batches each install's files are
/// split across N workers running in parallel threads — each owns its
/// own `Worker` (its own JS subprocess) and processes a contiguous slice
/// of them. Results are merged in input order so the output is
/// deterministic.
///
/// Worker count is chosen by [`pick_worker_count`] from input size and
/// `std::thread::available_parallelism`. Single-file or very-small
/// batches stay on one worker because the cost of spawning extra `bun`
/// / `node` processes (and re-importing `svelte/compiler` in each one)
/// outweighs the parallelism benefit.
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
    let runtime = pick_runtime()?;
    let groups = group_by_svelte(inputs.iter().map(|(p, _)| p.as_path()))?;
    let n_inputs = inputs.len();
    let mut slots: Vec<Option<(PathBuf, String)>> = inputs.into_iter().map(Some).collect();

    // Each install's files are split into contiguous chunks, keeping
    // their indexes so the merged result reads back in the caller's
    // original order.
    type Job = (PathBuf, Vec<(usize, (PathBuf, String))>);
    let mut jobs: Vec<Job> = Vec::new();
    for (pkg, members) in groups {
        let files: Vec<(usize, (PathBuf, String))> = members
            .into_iter()
            .filter_map(|i| Some((i, slots[i].take()?)))
            .collect();
        let n_workers = pick_worker_count(files.len());
        for chunk in split_into_chunks(files, n_workers) {
            if !chunk.is_empty() {
                jobs.push((pkg.clone(), chunk));
            }
        }
    }

    // Each chunk runs on its own thread, owning its own Worker. We
    // spawn workers from inside the thread (not on the main thread)
    // so the JS subprocess startup cost overlaps across workers — the
    // hot startup path is the `import svelte/compiler` inside each
    // bridge.mjs, which is single-threaded JS but runs concurrently
    // across processes.
    type ChunkResult = Result<Vec<(usize, (PathBuf, Vec<CompilerDiagnostic>))>, BridgeError>;
    let results: Vec<ChunkResult> = thread::scope(|scope| {
        let handles: Vec<_> = jobs
            .into_iter()
            .map(|(pkg, chunk)| {
                let runtime = &runtime;
                thread::Builder::new()
                    .name("svn-bridge-worker".into())
                    .spawn_scoped(scope, move || {
                        let (indexes, files): (Vec<usize>, Vec<(PathBuf, String)>) =
                            chunk.into_iter().unzip();
                        let out = compile_chunk(&pkg, workspace, runtime, files)?;
                        Ok(indexes.into_iter().zip(out).collect())
                    })
                    .map_err(BridgeError::Io)
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                // join() returns Err if the worker thread panicked; treat
                // as SubprocessDied since we have no diagnostics to fall
                // back on.
                h?.join().map_err(|_| BridgeError::SubprocessDied)?
            })
            .collect()
    });

    let mut ordered: Vec<Option<(PathBuf, Vec<CompilerDiagnostic>)>> =
        (0..n_inputs).map(|_| None).collect();
    for result in results {
        for (i, answer) in result? {
            ordered[i] = Some(answer);
        }
    }
    Ok(ordered.into_iter().flatten().collect())
}

/// [`Rejection`] for each file, from the JS runtime the bridge picks
/// (see the crate docs), or from Node when `on_node` is set. One worker
/// per `svelte` install: callers pass the few files they already expect
/// to fail. A file the bridge couldn't answer for is left out.
pub fn rejections(
    workspace: &Path,
    inputs: &[(PathBuf, String)],
    on_node: bool,
) -> Result<Vec<(PathBuf, Rejection)>, BridgeError> {
    if inputs.is_empty() {
        return Ok(Vec::new());
    }
    let runtime = if on_node {
        which_in_path("node").map_err(|_| BridgeError::NoRuntime)?
    } else {
        pick_runtime()?
    };
    rejections_on(workspace, inputs, &runtime)
}

/// Whether the JS runtime the bridge picks is Node.
pub fn runtime_is_node() -> bool {
    pick_runtime().is_ok_and(|path| path.file_stem().is_some_and(|stem| stem == "node"))
}

/// [`rejections`] on the JS runtime at `runtime`.
fn rejections_on(
    workspace: &Path,
    inputs: &[(PathBuf, String)],
    runtime: &Path,
) -> Result<Vec<(PathBuf, Rejection)>, BridgeError> {
    let mut out = Vec::with_capacity(inputs.len());
    for (pkg, members) in group_by_svelte(inputs.iter().map(|(p, _)| p.as_path()))? {
        let mut worker = Worker::spawn_on(&pkg, workspace, runtime)?;
        for (path, source) in members.into_iter().map(|i| &inputs[i]) {
            match worker.rejection(path, source) {
                Ok(rejection) => out.push((path.clone(), rejection)),
                Err(BridgeError::SubprocessDied) => return Err(BridgeError::SubprocessDied),
                Err(_) => {}
            }
        }
    }
    Ok(out)
}

/// Run a single chunk of inputs through one freshly-spawned `Worker` on
/// the `svelte` package `svelte_pkg`.
fn compile_chunk(
    svelte_pkg: &Path,
    workspace: &Path,
    runtime: &Path,
    inputs: Vec<(PathBuf, String)>,
) -> Result<Vec<(PathBuf, Vec<CompilerDiagnostic>)>, BridgeError> {
    let mut worker = Worker::spawn_on(svelte_pkg, workspace, runtime)?;
    let mut out = Vec::with_capacity(inputs.len());
    for (path, source) in inputs {
        match worker.compile_one(&path, &source) {
            Ok(d) => out.push((path, d)),
            Err(BridgeError::SubprocessDied) => return Err(BridgeError::SubprocessDied),
            // Writing to a worker that has gone fails with a broken
            // pipe before any read can find it dead.
            Err(BridgeError::Io(e)) if e.kind() == std::io::ErrorKind::BrokenPipe => {
                return Err(BridgeError::SubprocessDied);
            }
            Err(e) => {
                // One bad file doesn't kill the whole run: it gets an
                // empty result, and the reason is said.
                eprintln!(
                    "svelte-check-native: svelte/compiler gave no answer for {}: {e}",
                    path.display()
                );
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

/// One script as `ts.transpileModule` prints it.
#[derive(Debug, Clone, Deserialize)]
pub struct Transpiled {
    /// The JavaScript, with its `sourceMappingURL` comment.
    pub code: String,
    /// The `mappings` of its source map.
    pub mappings: String,
}

/// What became of one script given to [`transpile_scripts`].
#[derive(Debug, Clone)]
pub enum Transpile {
    /// TypeScript printed it.
    Printed(Transpiled),
    /// `transpileModule` threw on it.
    Threw,
    /// It could not be asked: no JS runtime, no `typescript` in the
    /// workspace that offers `transpileModule`, or the process given
    /// this script failed.
    NotRun,
}

/// Transpile each `(file name, TypeScript)` with the workspace's own
/// `typescript` package, exactly as svelte-check's fallback preprocessor
/// calls it. One entry per script, in order.
///
/// TypeScript prints on one thread, so the scripts are shared out over
/// several JS processes once there are enough of them to pay for
/// loading the package more than once. A process that fails costs only
/// its own scripts their answer.
pub fn transpile_scripts(workspace: &Path, scripts: &[(&Path, &str)]) -> Vec<Transpile> {
    /// Fewer scripts than this are not worth a process of their own.
    const PER_PROCESS: usize = 150;
    let not_run = || vec![Transpile::NotRun; scripts.len()];
    if scripts.is_empty() {
        return Vec::new();
    }
    let Ok(runtime) = pick_runtime() else {
        return not_run();
    };
    let Ok((_script_dir, script)) = write_script_to_temp("transpile", TRANSPILE_JS) else {
        return not_run();
    };
    // The script resolves `typescript` from here, which takes an
    // absolute path.
    let Ok(workspace) = std::path::absolute(workspace) else {
        return not_run();
    };
    let cores = thread::available_parallelism().map_or(1, usize::from);
    let processes = scripts.len().div_ceil(PER_PROCESS).clamp(1, cores);
    let per_process = scripts.len().div_ceil(processes);
    thread::scope(|scope| {
        let handles: Vec<_> = scripts
            .chunks(per_process)
            .map(|chunk| {
                let handle =
                    scope.spawn(|| transpile_in_one_process(&runtime, &script, &workspace, chunk));
                (chunk.len(), handle)
            })
            .collect();
        let mut printed = Vec::with_capacity(scripts.len());
        for (len, handle) in handles {
            match handle.join().ok().flatten() {
                Some(chunk) => printed.extend(chunk.into_iter().map(|p| match p {
                    Some(p) => Transpile::Printed(p),
                    None => Transpile::Threw,
                })),
                None => printed.extend(std::iter::repeat_n(Transpile::NotRun, len)),
            }
        }
        printed
    })
}

fn transpile_in_one_process(
    runtime: &Path,
    script: &Path,
    workspace: &Path,
    scripts: &[(&Path, &str)],
) -> Option<Vec<Option<Transpiled>>> {
    let request: Vec<serde_json::Value> = scripts
        .iter()
        .map(|(file, content)| {
            serde_json::json!({ "fileName": file.to_string_lossy(), "content": content })
        })
        .collect();
    let mut child = Command::new(runtime)
        .arg(script)
        .arg(workspace)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdin = child.stdin.take()?;
    let body = serde_json::Value::Array(request).to_string();
    // Written from its own thread: the child prints nothing until it
    // has read everything, but a full pipe must not stall this one.
    let writer = thread::spawn(move || {
        let _ = stdin.write_all(body.as_bytes());
    });
    let output = child.wait_with_output().ok()?;
    let _ = writer.join();
    if !output.status.success() {
        return None;
    }
    let printed: Option<Vec<Option<Transpiled>>> = serde_json::from_slice(&output.stdout).ok()?;
    printed.filter(|printed| printed.len() == scripts.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_line_escapes_line_separators() {
        let source = "{const\u{2028}1}\u{2029}";
        let line = request_line(&serde_json::json!({ "source": source }));
        assert!(!line.contains(['\u{2028}', '\u{2029}', '\n']));
        let back: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(back["source"], source);
    }

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

    #[cfg(unix)]
    #[test]
    fn scripts_are_written_to_a_directory_only_this_user_can_read() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, script) = write_script_to_temp("probe", "export {}").unwrap();
        assert_eq!(script.parent(), Some(dir.path()));
        let mode = std::fs::metadata(dir.path()).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o700);
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
