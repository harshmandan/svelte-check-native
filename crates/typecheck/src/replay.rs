//! No-change short-circuit for the tsgo subprocess.
//!
//! On a fully-warm run, tsgo's own incremental machinery still pays
//! its full config + parse cost before concluding from the buildinfo
//! that nothing needs re-checking — the buildinfo stores file hashes
//! and diagnostics, never ASTs, so a one-shot CLI invocation must
//! rebuild the whole program to validate it (identical behavior in
//! the JS compiler). That floor is ~0.5s on a 1350-component
//! workspace. The only way below it is to not spawn the compiler at
//! all: after a successful run we persist a fingerprint of every
//! input tsgo reads plus the raw diagnostics it printed; the next
//! run recomputes the fingerprint (parallel `stat`, no file reads)
//! and, on an exact match, replays the stored diagnostics into the
//! normal mapping pipeline. Everything downstream of the subprocess
//! (position mapping, filters, output formatting) still runs live.
//!
//! ## What the fingerprint covers, and why each part is load-bearing
//!
//! The invariant: if the fingerprint is unchanged, a real tsgo run
//! would print byte-identical diagnostics. Any input that could
//! change tsgo's output must therefore be visible here. A miss in
//! the CHANGED direction is safe (spurious subprocess run); a miss
//! in the UNCHANGED direction is a wrong result — when in doubt,
//! include more.
//!
//! - **Program files** — every path listed in the tsbuildinfo's
//!   `fileNames` (the closure tsgo actually loaded last run,
//!   including node_modules `.d.ts`), stat-compared by
//!   `(mtime, size)`. Catches edits to any file already in the
//!   program.
//! - **Include-root directory walks** — the buildinfo only lists
//!   files that EXISTED last run. A newly created file matched by an
//!   include glob joins the program without any listed file
//!   changing, so we walk the non-glob prefix of every `include`
//!   pattern (and stat non-glob include entries directly),
//!   collecting `(path, mtime, size)` for every program-candidate
//!   extension. Deletions surface the same way (the walk list
//!   shrinks). Roots whose path contains `node_modules` are walked
//!   fully (that's our own cache tree, re-included explicitly);
//!   elsewhere `node_modules` subdirs are skipped, mirroring
//!   TypeScript's always-on implicit exclusion.
//! - **`files` entries** — statted directly (shim, kit overlays,
//!   generated `.svn.ts` files that emit listed explicitly).
//! - **The overlay tsconfig text** — regenerated every run from the
//!   user's config + discovery output; hashing the exact text
//!   captures include/exclude/paths/rootDirs/files changes,
//!   including additions and removals of `.svelte` sources (they
//!   change the `files` array).
//! - **The user tsconfig `extends` chain** — tsgo re-reads these
//!   files itself at config time; a compiler-option edit changes
//!   diagnostics without touching any program file.
//! - **Ancestor `package.json` / lockfiles** — module RESOLUTION can
//!   change without any previously-loaded file changing (an install
//!   makes a previously-failing import resolve, adding new files to
//!   the program). Every install rewrites the manifest/lockfile, so
//!   their stats are the conservative proxy.
//! - **The compiler binary + flags** — engine path `(mtime, size)`,
//!   the flag-affecting inputs (`include_suggestions`, the
//!   `SVN_TSGO_*` env knobs), and our own crate version (a release
//!   may change what we ask of tsgo).
//!
//! `SVN_DISABLE_REPLAY` (any non-empty value) turns the whole layer
//! off — the safety valve if a fingerprint gap is ever suspected;
//! `--extendedDiagnostics` requests also bypass it (the stored run
//! has no timing block to replay).

use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};

use rayon::prelude::*;

use crate::cache::CacheLayout;
use crate::discovery::TsgoBinary;
use crate::output::RawDiagnostic;

/// Bump when the fingerprint structure or semantics change — a
/// mismatched schema is treated as no cache.
const SCHEMA: u32 = 1;

/// File extensions that can enter a TypeScript program through an
/// include-glob walk. Broader than strictly necessary — extra
/// entries only cause spurious re-runs, never wrong replays.
const PROGRAM_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "mts", "cts", "js", "jsx", "mjs", "cjs", "json", "svelte",
];

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct FileStat {
    path: String,
    /// Milliseconds since epoch; 0 when the platform gives no mtime.
    mtime_ms: u128,
    size: u64,
}

#[derive(Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Fingerprint {
    schema: u32,
    cli_version: String,
    tsgo: FileStat,
    include_suggestions: bool,
    /// Values of the `SVN_TSGO_*` runner knobs — they alter the
    /// spawned command line.
    tsgo_env: Vec<(String, String)>,
    overlay_tsconfig_hash: u64,
    chain: Vec<FileStat>,
    manifests: Vec<FileStat>,
    program: Vec<FileStat>,
    walked: Vec<FileStat>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ReplayCache {
    fingerprint: Fingerprint,
    diagnostics: Vec<RawDiagnostic>,
}

/// The computed current-state fingerprint plus the cache location.
/// Built once per check run, used for both the load and save side.
pub(crate) struct ReplayContext {
    cache_path: PathBuf,
    fingerprint: Fingerprint,
}

impl ReplayContext {
    /// Compute the current fingerprint. Returns `None` when replay is
    /// disabled (`SVN_DISABLE_REPLAY`), extended diagnostics were
    /// requested, or there is no buildinfo yet (first run — nothing
    /// to validate the program list against).
    pub(crate) fn compute(
        layout: &CacheLayout,
        overlay_text: &str,
        user_tsconfig: &Path,
        tsgo: &TsgoBinary,
        include_suggestions: bool,
        extended_diagnostics: bool,
    ) -> Option<Self> {
        if extended_diagnostics {
            return None;
        }
        if std::env::var("SVN_DISABLE_REPLAY").is_ok_and(|v| !v.is_empty()) {
            return None;
        }
        let program = program_stats(layout)?;
        let overlay: serde_json::Value = serde_json::from_str(overlay_text).ok()?;
        let walked = walk_include_roots(&overlay, layout);
        let mut hasher = std::hash::DefaultHasher::new();
        overlay_text.hash(&mut hasher);
        let fingerprint = Fingerprint {
            schema: SCHEMA,
            cli_version: env!("CARGO_PKG_VERSION").to_string(),
            tsgo: stat_file(&tsgo.path)?,
            include_suggestions,
            tsgo_env: ["SVN_TSGO_CHECKERS", "SVN_TSGO_SINGLE_THREADED"]
                .iter()
                .filter_map(|k| std::env::var(k).ok().map(|v| (k.to_string(), v)))
                .collect(),
            overlay_tsconfig_hash: hasher.finish(),
            chain: chain_stats(user_tsconfig),
            manifests: manifest_stats(&layout.workspace),
            program,
            walked,
        };
        Some(ReplayContext {
            cache_path: layout.root.join("replay.json"),
            fingerprint,
        })
    }

    /// Load the persisted cache and return its diagnostics iff the
    /// stored fingerprint matches the freshly computed one exactly.
    pub(crate) fn try_load(&self) -> Option<Vec<RawDiagnostic>> {
        let text = std::fs::read_to_string(&self.cache_path).ok()?;
        let cache: ReplayCache = serde_json::from_str(&text).ok()?;
        (cache.fingerprint == self.fingerprint).then_some(cache.diagnostics)
    }

    /// Persist this run's diagnostics under the computed fingerprint.
    /// Best-effort: a failed write only costs the next run its
    /// short-circuit.
    ///
    /// tsgo may have rewritten the buildinfo during the run just
    /// finished, so the program list is re-statted here rather than
    /// reusing the pre-run stats — otherwise the very next run would
    /// see the buildinfo-derived entries drift and never replay.
    pub(crate) fn save(mut self, layout: &CacheLayout, diagnostics: &[RawDiagnostic]) {
        let Some(program) = program_stats(layout) else {
            return;
        };
        self.fingerprint.program = program;
        let cache = ReplayCache {
            fingerprint: self.fingerprint,
            diagnostics: diagnostics.to_vec(),
        };
        if let Ok(text) = serde_json::to_string(&cache) {
            let _ = crate::cache::write_if_changed(&self.cache_path, &text);
        }
    }
}

fn stat_file(path: &Path) -> Option<FileStat> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() {
        return None;
    }
    let mtime_ms = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_millis())
        .unwrap_or(0);
    Some(FileStat {
        path: path.to_string_lossy().into_owned(),
        mtime_ms,
        size: meta.len(),
    })
}

/// Stat every file the last run's buildinfo lists. `None` when the
/// buildinfo is missing/unreadable (first run) — and note a listed
/// file that no longer exists is NOT a bail: it simply drops out of
/// the list, which changes the fingerprint (deletion detected).
fn program_stats(layout: &CacheLayout) -> Option<Vec<FileStat>> {
    let text = std::fs::read_to_string(&layout.tsbuildinfo).ok()?;
    let json: serde_json::Value = serde_json::from_str(&text).ok()?;
    let names = json.get("fileNames")?.as_array()?;
    let base = layout.tsbuildinfo.parent()?;
    let mut stats: Vec<FileStat> = names
        .par_iter()
        .filter_map(|v| v.as_str())
        .filter_map(|name| {
            // Bundled default-lib names (`lib.es2015.d.ts`) have no
            // path separator — they live inside the compiler, keyed
            // by the engine stat instead.
            if !name.contains('/') && !name.contains('\\') {
                return None;
            }
            let p = Path::new(name);
            let abs = if p.is_absolute() {
                p.to_path_buf()
            } else {
                base.join(p)
            };
            stat_file(&crate::path_utils::lexical_normalise(&abs))
        })
        .collect();
    stats.sort_by(|a, b| a.path.cmp(&b.path));
    Some(stats)
}

fn chain_stats(user_tsconfig: &Path) -> Vec<FileStat> {
    let mut stats: Vec<FileStat> = svn_core::tsconfig::load_chain(user_tsconfig)
        .unwrap_or_default()
        .iter()
        .filter_map(|f| stat_file(&f.path))
        .collect();
    stats.sort_by(|a, b| a.path.cmp(&b.path));
    stats
}

/// `package.json` + lockfiles at the workspace and every ancestor —
/// the conservative signal that module resolution may have changed.
fn manifest_stats(workspace: &Path) -> Vec<FileStat> {
    const NAMES: &[&str] = &[
        "package.json",
        "package-lock.json",
        "npm-shrinkwrap.json",
        "yarn.lock",
        "pnpm-lock.yaml",
        "bun.lock",
        "bun.lockb",
        "deno.lock",
    ];
    let mut stats = Vec::new();
    let mut dir = Some(workspace);
    while let Some(d) = dir {
        for name in NAMES {
            if let Some(s) = stat_file(&d.join(name)) {
                stats.push(s);
            }
        }
        dir = d.parent();
    }
    stats.sort_by(|a, b| a.path.cmp(&b.path));
    stats
}

/// Walk the non-glob prefix of every overlay `include` pattern and
/// stat every program-candidate file underneath, so files CREATED
/// since the last run are detected. Non-glob include entries are
/// statted as files. Roots contained in an already-walked root are
/// skipped.
fn walk_include_roots(overlay: &serde_json::Value, layout: &CacheLayout) -> Vec<FileStat> {
    let Some(includes) = overlay.get("include").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut roots: Vec<PathBuf> = Vec::new();
    let mut single_files: Vec<PathBuf> = Vec::new();
    for pat in includes.iter().filter_map(|v| v.as_str()) {
        match glob_prefix(pat) {
            GlobPrefix::Dir(root) => {
                if !roots.iter().any(|r| root.starts_with(r)) {
                    roots.retain(|r| !r.starts_with(&root));
                    roots.push(root);
                }
            }
            GlobPrefix::File(f) => single_files.push(f),
        }
    }
    let mut stats: Vec<FileStat> = roots
        .par_iter()
        .flat_map_iter(|root| {
            // Our cache tree lives under `node_modules/.cache/` and is
            // re-included explicitly, so it walks fully; everywhere
            // else `node_modules` is implicitly excluded by
            // TypeScript regardless of the exclude list. `.git` never
            // contributes program files.
            let walk_node_modules = root.components().any(|c| c.as_os_str() == "node_modules");
            walkdir::WalkDir::new(root)
                .into_iter()
                .filter_entry(move |e| {
                    let name = e.file_name();
                    if name == ".git" {
                        return false;
                    }
                    !(e.file_type().is_dir() && !walk_node_modules && name == "node_modules")
                })
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter(|e| {
                    e.path()
                        .extension()
                        .and_then(|s| s.to_str())
                        .is_some_and(|ext| PROGRAM_EXTENSIONS.contains(&ext))
                })
                .filter_map(|e| stat_file(e.path()))
                .collect::<Vec<_>>()
        })
        .collect();
    stats.extend(single_files.iter().filter_map(|f| stat_file(f)));
    // The buildinfo itself changes only when the program did; keying
    // on it here costs at most one spurious run after tsgo rewrites
    // it, and covers anything the walk heuristics might miss about
    // tsgo's own view of the program.
    stats.extend(stat_file(&layout.tsbuildinfo));
    stats.sort_by(|a, b| a.path.cmp(&b.path));
    stats.dedup();
    stats
}

enum GlobPrefix {
    Dir(PathBuf),
    File(PathBuf),
}

/// Longest directory prefix of an include pattern before the first
/// glob metacharacter; patterns without metacharacters are single
/// files (TypeScript treats a non-glob include as a file reference).
fn glob_prefix(pattern: &str) -> GlobPrefix {
    match pattern.find(['*', '?', '{', '[']) {
        None => GlobPrefix::File(PathBuf::from(pattern)),
        Some(idx) => {
            let prefix = &pattern[..idx];
            let dir = match prefix.rfind('/') {
                Some(slash) => &prefix[..slash],
                None => prefix,
            };
            GlobPrefix::Dir(PathBuf::from(dir))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn glob_prefix_splits_at_first_metachar() {
        let GlobPrefix::Dir(d) = glob_prefix("/ws/src/**/*.ts") else {
            panic!("expected dir");
        };
        assert_eq!(d, Path::new("/ws/src"));
        let GlobPrefix::Dir(d) = glob_prefix("/ws/src/routes/[slug]/x.ts") else {
            panic!("expected dir");
        };
        assert_eq!(d, Path::new("/ws/src/routes"));
        let GlobPrefix::File(f) = glob_prefix("/ws/.svelte-kit/ambient.d.ts") else {
            panic!("expected file");
        };
        assert_eq!(f, Path::new("/ws/.svelte-kit/ambient.d.ts"));
    }

    #[test]
    fn fingerprint_roundtrips_and_detects_change() {
        let tmp = tempfile::tempdir().unwrap();
        let f = tmp.path().join("a.ts");
        std::fs::write(&f, "export const a = 1;").unwrap();
        let s1 = stat_file(&f).unwrap();
        let s2 = stat_file(&f).unwrap();
        assert_eq!(s1, s2);
        // A size change is always visible even when mtime granularity
        // hides a same-instant rewrite.
        std::fs::write(&f, "export const a = 12;").unwrap();
        let s3 = stat_file(&f).unwrap();
        assert_ne!(s1, s3);
    }

    #[test]
    fn missing_file_stats_as_none() {
        assert!(stat_file(Path::new("/nonexistent/replay/probe.ts")).is_none());
    }
}
