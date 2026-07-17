//! Workspace file discovery.
//!
//! Single-pass walk of the user's workspace producing the four file
//! categories the typecheck pipeline cares about: Svelte components,
//! SvelteKit route/hooks files, `.svelte.ts` runes modules, and plain
//! `.ts` files (candidates for the runes-collision overlay rewrite).
//!
//! Also hosts the small predicates that govern walk pruning
//! (`is_excluded_dir`, `path_is_under_node_modules`) and the tsconfig
//! `include`/`exclude` glob compiler used by the project-scope filter.

use std::path::{Path, PathBuf};

use svn_core::sveltekit::{KitFilesSettings, classify};
use walkdir::WalkDir;

/// Does `path` contain a `node_modules` segment? Uses path components
/// (not string-contains) so a directory named `my_node_modules_dir`
/// doesn't trip the check.
pub(crate) fn path_is_under_node_modules(path: &Path) -> bool {
    path.components().any(
        |c| matches!(c, std::path::Component::Normal(name) if name == svn_core::NODE_MODULES_DIR),
    )
}

/// Convenience wrapper for callers that only need the `.svelte` file
/// list (e.g. `--emit-ts`, `--list-relevant`). Uses default Kit-file
/// settings — fine for these debug flows since `.svelte` discovery
/// doesn't consult them.
pub(crate) fn discover_svelte_files(workspace: &Path) -> Vec<PathBuf> {
    discover_relevant_files_with_settings(workspace, &KitFilesSettings::default()).0
}

/// Wrapper accepting default Kit-file settings — kept for callers
/// (notably the `--list-relevant` debug flow) that don't have the
/// user's `svelte.config.js` parsed yet.
pub(crate) fn discover_relevant_files(
    workspace: &Path,
) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>) {
    discover_relevant_files_with_settings(workspace, &KitFilesSettings::default())
}

/// Walk the workspace once and return all four file categories the
/// typecheck pipeline consumes:
///
/// 1. `.svelte` components.
/// 2. SvelteKit route/hooks files (`+page.ts`, `+layout.ts`, etc.).
/// 3. `.svelte.ts` runes modules (separately tracked so the runes-
///    collision overlay decider can O(1) membership-test).
/// 4. Plain `.ts` files (candidates for `.svelte`-import rewriting
///    when their imports collide with a sibling runes module).
///
/// Sharing the walker pass means callers that need multiple
/// categories don't traverse the filesystem more than once.
///
/// Kit-file detection uses `KitFilesSettings::default()` — the
/// `kit.files` overrides in `svelte.config.js` aren't parsed yet
/// (defaults cover the overwhelming majority of projects; overrides
/// would require evaluating JS). Not a correctness issue for the
/// denominator; files processed by tsgo via `include` globs are the
/// same either way.
pub(crate) fn discover_relevant_files_with_settings(
    workspace: &Path,
    kit_settings: &KitFilesSettings,
) -> (Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>, Vec<PathBuf>) {
    let mut svelte_files = Vec::new();
    let mut kit_files = Vec::new();
    // `.svelte.ts` and `.svelte.js` runes modules — siblings of a
    // `.svelte` component, the pattern that creates the rootDirs
    // resolution collision fixed by user-script overlays. Collected
    // here once so the overlay decider can membership-test without
    // rewalking disk. Both lang variants live in the same set
    // because the collision is identical and the rewrite output
    // (`.svelte.svn.js`) is the same regardless of source lang.
    let mut runes_modules = Vec::new();
    // User `.ts` and `.js` files that aren't Kit files and aren't
    // runes modules. Candidates for the `.svelte`-import-rewrite
    // overlay — final filter (does the file actually import a
    // sibling-collision `.svelte`?) happens later after all runes
    // modules are known.
    let mut user_scripts = Vec::new();
    for e in WalkDir::new(workspace)
        .into_iter()
        // depth 0 is the workspace root itself — never prune it, even
        // if its basename is hidden or `node_modules` (the user pointed
        // us at it deliberately). Pruning the root yields zero files.
        .filter_entry(|e| e.depth() == 0 || !e.file_type().is_dir() || !is_excluded_dir(e.path()))
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = e.path();
        let ext = path.extension().and_then(|s| s.to_str());
        let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
        match ext {
            Some("svelte") => svelte_files.push(path.to_path_buf()),
            // Any classify hit on a `.ts`/`.js` is a kit file — route
            // components are `.svelte` and never reach this branch.
            // See `svn_core::sveltekit::classify`.
            Some("ts" | "js") if classify(path, kit_settings).is_some() => {
                kit_files.push(path.to_path_buf());
            }
            Some("ts" | "js")
                if file_name.ends_with(".svelte.ts") || file_name.ends_with(".svelte.js") =>
            {
                runes_modules.push(path.to_path_buf());
            }
            Some("ts" | "js") => user_scripts.push(path.to_path_buf()),
            _ => {}
        }
    }
    (svelte_files, kit_files, runes_modules, user_scripts)
}

/// Lexically normalize a path, collapsing `.` and `..` segments
/// without touching the filesystem. Mirrors the overlay builder's
/// `normalize` so discovery and the overlay resolve include/exclude
/// the same way.
fn normalize_lexical(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Resolve the chain's winning `include`/`exclude`/`files` patterns
/// (TS `extends` precedence: leaf wins, later array-extends entries
/// beat earlier ones — see [`svn_core::tsconfig::winning_patterns`])
/// against the DECLARING config's directory — TypeScript resolves
/// these fields relative to the config file that declares them.
/// Returns absolute, lexically-normalized pattern strings.
///
/// `None` means no config in the chain declares the field; an explicit
/// empty array is a declaration and yields `Some(vec![])` — for
/// `include` that means "no files admitted via include", NOT "default
/// to everything" (TS replace-on-child semantics).
///
/// This mirrors the overlay builder's `winning_patterns_absolute`
/// exactly, so the file-scope filter here and the overlay's `include`
/// projection agree on which files are in the project.
pub(crate) fn resolve_patterns_against_declaring_dir<F>(
    chain: &[svn_core::tsconfig::TsConfigFile],
    get: F,
) -> Option<Vec<String>>
where
    F: for<'a> Fn(&'a svn_core::tsconfig::TsConfigFile) -> Option<&'a [String]>,
{
    let (winner, patterns) = svn_core::tsconfig::winning_patterns(chain, get)?;
    let dir = winner.config_dir();
    Some(
        patterns
            .iter()
            .map(|s| {
                let resolved = if Path::new(s).is_absolute() {
                    PathBuf::from(s)
                } else {
                    dir.join(s)
                };
                normalize_lexical(&resolved).to_string_lossy().into_owned()
            })
            .collect(),
    )
}

/// Is the filesystem holding `probe` case-insensitive? Mirrors
/// TypeScript's `sys.isFileSystemCaseSensitive` (sys.ts): Windows is
/// always insensitive; elsewhere, swap the case of an existing path's
/// letters and test whether the swapped spelling still exists. TS
/// probes its own `__filename`; we probe the caller-supplied path
/// (the tsconfig — guaranteed to exist and to sit on the same mount
/// as the files being matched). A probe with no letters to flip, or
/// one that doesn't exist, reports case-SENSITIVE — same conservative
/// default as TS's `swapCase` no-op path.
pub(crate) fn path_fs_is_case_insensitive(probe: &Path) -> bool {
    if cfg!(windows) {
        return true;
    }
    let Some(name) = probe.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let swapped: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_uppercase() {
                c.to_ascii_lowercase()
            } else if c.is_ascii_lowercase() {
                c.to_ascii_uppercase()
            } else {
                c
            }
        })
        .collect();
    if swapped == name || !probe.exists() {
        return false;
    }
    probe.with_file_name(swapped).exists()
}

/// Build a [`globset::GlobSet`] from include/exclude patterns already
/// resolved to absolute paths (via
/// [`resolve_patterns_against_declaring_dir`]). Matched against
/// absolute file paths.
/// Unparseable patterns are dropped (TS tolerates minor config typos).
///
/// Mirrors TypeScript's `getSubPatternFromSpec` matcher semantics
/// (utilities.ts), which is what tsgo applies when it resolves the
/// overlay tsconfig itself — divergence here makes our denominator /
/// kit decisions disagree with the file set tsgo actually checks:
///
/// - **Implicit glob expansion is lexical** (`isImplicitGlob`): a
///   pattern whose LAST component contains none of `.` `*` `?` gets
///   `/**/*` appended — whether or not the directory exists on disk.
///   TS never stats the path; neither do we. (Corollary: an include
///   naming a directory with a dot in its name, e.g. `"src/v1.2"`,
///   is treated as a file pattern by TS and matches nothing under
///   it.)
/// - **Only `*` and `?` are wildcards.** TS treats `[` and `{`
///   literally, but globset would parse a character class /
///   alternation — escape them before compiling.
/// - `literal_separator(true)`: `*` and `?` match within a single
///   path segment and never cross a `/`; only `**` descends.
///   globset's default would let `src/*` match the whole `src/`
///   subtree.
/// - `case_insensitive`: TS builds its matcher regexes with the `i`
///   flag when the host reports `useCaseSensitiveFileNames: false`
///   (default on macOS/Windows). Callers pass the
///   [`path_fs_is_case_insensitive`] probe result so a
///   case-mismatched `include` (e.g. `"Src/**/*"` on disk-`src`)
///   still scopes the same files tsgo checks.
pub(crate) fn build_glob_set_absolute(
    patterns: &[String],
    case_insensitive: bool,
) -> Option<globset::GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = globset::GlobSetBuilder::new();
    let mut any = false;
    for pat in patterns {
        // Escape globset metacharacters that are literal in TS specs.
        // Order matters: `[` first (so classes introduced by the later
        // replacements aren't re-escaped); `}` too, since globset
        // errors on an unopened alternate group once `{` is escaped.
        let mut p = pat
            .replace('[', "[[]")
            .replace('{', "[{]")
            .replace('}', "[}]");
        let last_component = p
            .trim_end_matches(['/', '\\'])
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or("");
        if !last_component.is_empty() && !last_component.contains(['.', '*', '?'])
        // `[[]` / `[{]` are escapes of literal `[` / `{`, which TS's
        // isImplicitGlob doesn't treat as wildcards either — but a
        // component containing them still has no `.`/`*`/`?`, so the
        // check above already handles them correctly.
        {
            while p.ends_with('/') || p.ends_with('\\') {
                p.pop();
            }
            p.push_str("/**/*");
        }
        if let Ok(glob) = globset::GlobBuilder::new(&p)
            .literal_separator(true)
            .case_insensitive(case_insensitive)
            .build()
        {
            builder.add(glob);
            any = true;
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}

/// Directory names the discovery walker never descends into.
///
/// Matches upstream svelte-check's `findFiles` exactly (utils.ts): it
/// prunes only `node_modules` and hidden (`.`-prefixed) directories —
/// the latter covers `.git`, `.svelte-kit`, and our `.svelte-check`
/// cache. We deliberately do NOT exclude `dist`/`target`: upstream
/// descends into them, and a project that ships checkable `.svelte`
/// sources under `dist/` must see the same `<N> FILES` denominator
/// (the project's stated parity bar). Like upstream (post-#3034), we
/// prune by directory NAME rather than full path, so a workspace
/// nested under a hidden ancestor directory still discovers its files.
///
/// NOTE: callers must NOT apply this to the walk's ROOT entry — a
/// workspace whose own basename starts with `.` (or is literally
/// `node_modules`) is a legitimate target and pruning it discovers
/// zero files. Gate on `entry.depth() == 0` at the call site.
pub(crate) fn is_excluded_dir(path: &Path) -> bool {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    name == "node_modules" || name.starts_with('.')
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::*;

    #[test]
    fn node_modules_filter_matches_component_not_substring() {
        assert!(path_is_under_node_modules(Path::new(
            "/app/node_modules/pkg/Foo.svelte"
        )));
        assert!(path_is_under_node_modules(Path::new(
            "/app/packages/foo/node_modules/pkg/Foo.svelte"
        )));
        assert!(!path_is_under_node_modules(Path::new(
            "/app/src/my_node_modules_dir/Foo.svelte"
        )));
        assert!(!path_is_under_node_modules(Path::new(
            "/app/src/routes/+page.svelte"
        )));
    }

    #[test]
    fn excluded_dirs_match_upstream_node_modules_and_hidden_only() {
        // Pruned: node_modules + any hidden dir (covers .git/.svelte-kit/
        // .svelte-check).
        assert!(is_excluded_dir(Path::new("/app/node_modules")));
        assert!(is_excluded_dir(Path::new("/app/.git")));
        assert!(is_excluded_dir(Path::new("/app/.svelte-kit")));
        assert!(is_excluded_dir(Path::new("/app/.svelte-check")));
        // NOT pruned: dist/target — upstream descends into them, so we
        // must too (FILES-denominator parity).
        assert!(!is_excluded_dir(Path::new("/app/dist")));
        assert!(!is_excluded_dir(Path::new("/app/target")));
        assert!(!is_excluded_dir(Path::new("/app/src")));
    }

    #[test]
    fn normalize_lexical_collapses_parent_segments() {
        assert_eq!(
            normalize_lexical(Path::new("/ws/.svelte-kit/../src/**/*.svelte")),
            PathBuf::from("/ws/src/**/*.svelte")
        );
        assert_eq!(
            normalize_lexical(Path::new("/ws/./a/./b")),
            PathBuf::from("/ws/a/b")
        );
    }

    #[test]
    fn resolve_patterns_last_array_extends_entry_wins_against_its_own_dir() {
        // extends: ["./a.json", "./sub/b.json"] with include declared in
        // both parents and not the leaf: TS gives the LAST entry
        // precedence, and resolves its patterns against ITS directory.
        let tmp = tempfile::tempdir().expect("tempdir");
        let sub = tmp.path().join("sub");
        std::fs::create_dir_all(&sub).expect("mkdir");
        std::fs::write(tmp.path().join("a.json"), r#"{ "include": ["app/**/*"] }"#).expect("write");
        std::fs::write(sub.join("b.json"), r#"{ "include": ["src/**/*"] }"#).expect("write");
        let leaf = tmp.path().join("tsconfig.json");
        std::fs::write(&leaf, r#"{ "extends": ["./a.json", "./sub/b.json"] }"#).expect("write");

        let chain = svn_core::tsconfig::load_chain(&leaf).expect("chain");
        let include = resolve_patterns_against_declaring_dir(&chain, |f| f.include.as_deref())
            .expect("include is declared in the chain");
        let expected = normalize_lexical(
            &dunce::canonicalize(&sub)
                .expect("canonicalize")
                .join("src/**/*"),
        );
        assert_eq!(include, [expected.to_string_lossy().into_owned()]);
    }

    #[test]
    fn resolve_patterns_distinguishes_explicit_empty_from_undeclared() {
        let tmp = tempfile::tempdir().expect("tempdir");
        std::fs::write(
            tmp.path().join("base.json"),
            r#"{ "include": ["src/**/*"] }"#,
        )
        .expect("write");
        let leaf = tmp.path().join("tsconfig.json");
        std::fs::write(&leaf, r#"{ "extends": "./base.json", "include": [] }"#).expect("write");

        let chain = svn_core::tsconfig::load_chain(&leaf).expect("chain");
        // Explicit `"include": []` REPLACES the parent's include:
        // declared-but-empty, not "fall through to the parent".
        assert_eq!(
            resolve_patterns_against_declaring_dir(&chain, |f| f.include.as_deref()),
            Some(Vec::new())
        );
        // `exclude` is declared nowhere → None.
        assert_eq!(
            resolve_patterns_against_declaring_dir(&chain, |f| f.exclude.as_deref()),
            None
        );
    }

    #[test]
    fn glob_star_never_crosses_a_directory_separator() {
        // TypeScript's include/exclude wildcard rules: `*` and `?` match
        // within one path segment only; only `**` descends. globset's
        // default lets `*` cross `/`, which would scope `src/*` over the
        // whole subtree.
        let set = build_glob_set_absolute(&["/ws/src/*".to_string()], false).expect("glob set");
        assert!(set.is_match(Path::new("/ws/src/Foo.svelte")));
        assert!(!set.is_match(Path::new("/ws/src/lib/deep/Foo.svelte")));

        let stories = build_glob_set_absolute(&["/ws/src/*.stories.svelte".to_string()], false)
            .expect("glob set");
        assert!(stories.is_match(Path::new("/ws/src/A.stories.svelte")));
        assert!(!stories.is_match(Path::new("/ws/src/nested/A.stories.svelte")));

        let single =
            build_glob_set_absolute(&["/ws/src/?.svelte".to_string()], false).expect("glob set");
        assert!(single.is_match(Path::new("/ws/src/A.svelte")));
        assert!(!single.is_match(Path::new("/ws/src/a/b.svelte")));

        // `**` still crosses directories.
        let rec =
            build_glob_set_absolute(&["/ws/src/**/*.svelte".to_string()], false).expect("glob set");
        assert!(rec.is_match(Path::new("/ws/src/lib/deep/Foo.svelte")));
        assert!(rec.is_match(Path::new("/ws/src/Foo.svelte")));
    }

    #[test]
    fn glob_set_absolute_matches_absolute_paths_and_expands_bare_dirs() {
        // A glob pattern matches by absolute path.
        let set =
            build_glob_set_absolute(&["/ws/src/**/*.svelte".to_string()], false).expect("glob set");
        assert!(set.is_match(Path::new("/ws/src/lib/Foo.svelte")));
        assert!(!set.is_match(Path::new("/other/src/Foo.svelte")));
    }

    #[test]
    fn implicit_glob_expansion_is_lexical_like_typescript() {
        // TS's isImplicitGlob never stats the disk: a last component
        // with no `.` / `*` / `?` gets `/**/*` appended even when the
        // directory doesn't exist.
        let bare =
            build_glob_set_absolute(&["/ws/does-not-exist".to_string()], false).expect("set");
        assert!(bare.is_match(Path::new("/ws/does-not-exist/Foo.svelte")));
        assert!(bare.is_match(Path::new("/ws/does-not-exist/deep/Foo.svelte")));
        // ...and the bare name itself no longer matches as a file
        // (TS matches `src` as `src/**/*`, which excludes `src`).
        assert!(!bare.is_match(Path::new("/ws/does-not-exist")));

        // A trailing separator is tolerated.
        let trailing = build_glob_set_absolute(&["/ws/src/".to_string()], false).expect("set");
        assert!(trailing.is_match(Path::new("/ws/src/Foo.svelte")));

        // Conversely, a REAL directory whose name contains a dot is
        // treated as a file pattern (TS quirk, mirrored on purpose).
        let tmp = tempfile::tempdir().expect("tempdir");
        let dotted = tmp.path().join("v1.2");
        std::fs::create_dir(&dotted).expect("mkdir");
        std::fs::write(dotted.join("Foo.svelte"), "x").expect("write");
        let set =
            build_glob_set_absolute(&[dotted.to_string_lossy().into_owned()], false).expect("set");
        assert!(
            !set.is_match(dotted.join("Foo.svelte")),
            "a dotted directory include must NOT expand recursively (TS treats it as a file spec)"
        );
    }

    #[test]
    fn bracket_and_brace_are_literal_like_typescript() {
        // TS's only wildcards are `*` and `?`; `[` and `{` are literal
        // path characters. globset would parse a character class /
        // alternation, silently matching different files than tsgo.
        let set = build_glob_set_absolute(&["/ws/[app]/*.svelte".to_string()], false).expect("set");
        assert!(set.is_match(Path::new("/ws/[app]/Foo.svelte")));
        assert!(!set.is_match(Path::new("/ws/a/Foo.svelte")));

        let braces =
            build_glob_set_absolute(&["/ws/{a,b}/*.svelte".to_string()], false).expect("set");
        assert!(braces.is_match(Path::new("/ws/{a,b}/Foo.svelte")));
        assert!(!braces.is_match(Path::new("/ws/a/Foo.svelte")));
    }

    #[test]
    fn case_insensitive_flag_matches_mismatched_case() {
        let pats = vec!["/ws/Src/**/*.svelte".to_string()];
        let sensitive = build_glob_set_absolute(&pats, false).expect("set");
        assert!(!sensitive.is_match(Path::new("/ws/src/Foo.svelte")));
        let insensitive = build_glob_set_absolute(&pats, true).expect("set");
        assert!(insensitive.is_match(Path::new("/ws/src/Foo.svelte")));
        assert!(insensitive.is_match(Path::new("/ws/SRC/FOO.SVELTE")));
    }

    #[test]
    fn fs_case_probe_agrees_with_direct_observation() {
        // Self-differential: create a file, then compare our probe
        // with what the filesystem actually reports for the
        // case-swapped spelling. Runs correctly on both sensitive
        // (Linux CI) and insensitive (default macOS/Windows) disks.
        let tmp = tempfile::tempdir().expect("tempdir");
        let probe = tmp.path().join("CaseProbe.cfg");
        std::fs::write(&probe, "x").expect("write");
        let observed = tmp.path().join("cASEpROBE.CFG").exists();
        assert_eq!(path_fs_is_case_insensitive(&probe), observed);
        // A non-existent probe reports case-sensitive (conservative).
        assert!(!path_fs_is_case_insensitive(&tmp.path().join("missing")));
    }

    #[test]
    fn include_case_mismatch_scopes_files_on_case_insensitive_fs() {
        // The end-to-end shape of the bug: tsconfig `include` spells a
        // directory with different case than the disk. tsgo (which
        // honours useCaseSensitiveFileNames) checks the files; our
        // scope filter must agree. Assertion gated on the temp dir's
        // filesystem actually being case-insensitive so Linux CI
        // exercises the sensitive branch instead.
        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = tmp.path();
        std::fs::create_dir(ws.join("src")).expect("mkdir");
        std::fs::write(ws.join("src/App.svelte"), "<p>x</p>").expect("write");
        let tsconfig = ws.join("tsconfig.json");
        std::fs::write(&tsconfig, "{}").expect("write");

        let case_insensitive = path_fs_is_case_insensitive(&tsconfig);
        let pats = vec![ws.join("Src/**/*").to_string_lossy().into_owned()];
        let set = build_glob_set_absolute(&pats, case_insensitive).expect("set");
        assert_eq!(
            set.is_match(ws.join("src/App.svelte")),
            case_insensitive,
            "scope filter must match the file exactly when the FS (and thus tsgo) does"
        );
    }

    /// Differential lock against the real engine: the file set our
    /// include matcher admits equals what tsgo resolves for the same
    /// tsconfig (`--listFilesOnly`). Skips silently when the dev-local
    /// tsgo install (repo-root node_modules) is missing.
    #[test]
    fn include_matcher_agrees_with_tsgo_list_files() {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let tsgo = repo_root.join("node_modules/.bin/tsgo");
        if !tsgo.exists() {
            eprintln!("SKIP: no dev-local tsgo at {}", tsgo.display());
            return;
        }

        let tmp = tempfile::tempdir().expect("tempdir");
        let ws = dunce::canonicalize(tmp.path()).expect("canonicalize");
        for f in [
            "src/a.ts",
            "src/deep/b.ts",
            "SRC/upper.ts", // same dir as src/ on case-insensitive disks
            "other/c.ts",
            "app.dir/d.ts", // dotted dir: TS treats `app.dir` as a file spec
        ] {
            let p = ws.join(f);
            std::fs::create_dir_all(p.parent().expect("parent")).expect("mkdir");
            std::fs::write(&p, "export {};\n").expect("write");
        }
        // `missing` doesn't exist on disk — lexical expansion must not
        // error or change the resolved set.
        std::fs::write(
            ws.join("tsconfig.json"),
            r#"{
                "compilerOptions": { "noEmit": true, "types": [] },
                "include": ["Src", "app.dir", "missing"]
            }"#,
        )
        .expect("write tsconfig");

        let out = std::process::Command::new(&tsgo)
            .args(["--listFilesOnly", "-p"])
            .arg(&ws)
            .output()
            .expect("tsgo runs");
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut tsgo_files: Vec<String> = stdout
            .lines()
            .map(str::trim)
            .filter(|l| l.starts_with(ws.to_string_lossy().as_ref()))
            .map(|l| l.to_string())
            .collect();
        tsgo_files.sort();

        let tsconfig = ws.join("tsconfig.json");
        let case_insensitive = path_fs_is_case_insensitive(&tsconfig);
        let pats = vec![
            ws.join("Src").to_string_lossy().into_owned(),
            ws.join("app.dir").to_string_lossy().into_owned(),
            ws.join("missing").to_string_lossy().into_owned(),
        ];
        let set = build_glob_set_absolute(&pats, case_insensitive).expect("set");
        let mut ours: Vec<String> = walkdir::WalkDir::new(&ws)
            .into_iter()
            .filter_map(Result::ok)
            .filter(|e| e.file_type().is_file())
            .map(|e| e.path().to_string_lossy().into_owned())
            .filter(|p| p.ends_with(".ts") && set.is_match(Path::new(p)))
            .collect();
        ours.sort();

        assert_eq!(
            ours, tsgo_files,
            "our include matcher and tsgo resolved different file sets"
        );
    }
}
