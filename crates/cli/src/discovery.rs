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
        .filter_entry(|e| !is_excluded_dir(e.path()))
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

/// Resolve the first config in `chain` that declares the patterns
/// returned by `get`, against THAT config's directory — TypeScript
/// resolves `include`/`exclude` relative to the config file that
/// declares them, and the leaf wins (replace-on-child). Returns
/// absolute, lexically-normalized pattern strings.
///
/// This mirrors the overlay builder's `first_non_empty_patterns`
/// exactly, so the file-scope filter here and the overlay's `include`
/// projection agree on which files are in the project — the two used
/// to disagree (overlay resolved against the declaring dir; discovery
/// resolved the *flattened* patterns against the workspace via a
/// leading-`../`-stripping heuristic that's wrong for configs declared
/// outside the workspace root).
pub(crate) fn resolve_patterns_against_declaring_dir<F>(
    chain: &[svn_core::tsconfig::TsConfigFile],
    get: F,
) -> Vec<String>
where
    F: Fn(&svn_core::tsconfig::TsConfigFile) -> Option<&[String]>,
{
    for file in chain {
        let Some(patterns) = get(file) else {
            continue;
        };
        if patterns.is_empty() {
            continue;
        }
        let dir = file.config_dir();
        return patterns
            .iter()
            .map(|s| {
                let resolved = if Path::new(s).is_absolute() {
                    PathBuf::from(s)
                } else {
                    dir.join(s)
                };
                normalize_lexical(&resolved).to_string_lossy().into_owned()
            })
            .collect();
    }
    Vec::new()
}

/// Build a [`globset::GlobSet`] from include/exclude patterns already
/// resolved to absolute paths (via
/// [`resolve_patterns_against_declaring_dir`]). Matched against
/// absolute file paths. A bare directory (no glob metacharacters)
/// expands to a recursive `/**/*` — TS treats `"src"` as "all files
/// under src/", whereas globset would read it as a literal name match.
/// Unparseable patterns are dropped (TS tolerates minor config typos).
pub(crate) fn build_glob_set_absolute(patterns: &[String]) -> Option<globset::GlobSet> {
    if patterns.is_empty() {
        return None;
    }
    let mut builder = globset::GlobSetBuilder::new();
    let mut any = false;
    for pat in patterns {
        let mut p = pat.clone();
        if !p.contains('*') && !p.contains('?') && !p.contains('[') {
            if Path::new(&p).is_dir() {
                if !p.ends_with('/') {
                    p.push('/');
                }
                p.push_str("**/*");
            }
        }
        if let Ok(glob) = globset::Glob::new(&p) {
            builder.add(glob);
            any = true;
        }
    }
    if !any {
        return None;
    }
    builder.build().ok()
}


/// Hard-coded directory names the discovery walker never descends
/// into. Covers npm/yarn/pnpm/bun caches, build outputs, version-
/// control metadata, and our own cache directory. Anything starting
/// with `.` is skipped too (treats hidden directories as out of
/// scope, matching upstream svelte-check's behaviour and the rare
/// real-world workspace where `.something` is meaningful).
pub(crate) fn is_excluded_dir(path: &Path) -> bool {
    let name = match path.file_name().and_then(|s| s.to_str()) {
        Some(n) => n,
        None => return false,
    };
    matches!(
        name,
        "node_modules" | ".git" | ".svelte-kit" | ".svelte-check" | "target" | "dist"
    ) || name.starts_with('.')
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
    fn glob_set_absolute_matches_absolute_paths_and_expands_bare_dirs() {
        // A glob pattern matches by absolute path.
        let set =
            build_glob_set_absolute(&["/ws/src/**/*.svelte".to_string()]).expect("glob set");
        assert!(set.is_match(Path::new("/ws/src/lib/Foo.svelte")));
        assert!(!set.is_match(Path::new("/other/src/Foo.svelte")));
        // A non-existent bare dir stays a literal (no /**/* expansion,
        // since is_dir is false) — it simply won't match nested files.
        let bare = build_glob_set_absolute(&["/ws/does-not-exist".to_string()]).expect("set");
        assert!(!bare.is_match(Path::new("/ws/does-not-exist/Foo.svelte")));
    }
}
