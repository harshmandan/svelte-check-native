//! Generate the overlay tsconfig that tsgo runs against.
//!
//! The overlay extends the user's tsconfig and re-points it at the
//! generated `.svelte.ts` files in the cache. Forces the flags tsgo needs
//! to consume our virtual files (`allowArbitraryExtensions`, `noEmit`,
//! incremental build info location).
//!
//! ### rootDirs merging
//!
//! TS treats `rootDirs` as an array — and arrays do NOT merge across the
//! `extends` chain (inner config wins outright). That means just setting
//! a child `rootDirs` here would clobber whatever the user's tsconfig had
//! (commonly SvelteKit's `[".." , "./types"]`).
//!
//! To keep relative imports resolving from generated `++Foo.svelte.ts`
//! files in the overlay back to the original source tree, we compute the
//! union of:
//!   - `<overlay>/svelte`        — where our generated `.ts` files live
//!   - every `rootDirs` entry from the user's `extends` chain (resolved
//!     to absolute paths so the absolute-vs-relative distinction is gone)
//!   - the workspace root         — fallback for projects that don't
//!     declare `rootDirs` themselves
//!
//! TS then virtually merges all those folders, so a relative import
//! `./loading-labels` from
//! `<overlay>/svelte/src/lib/components/ai/++AssistantOverlay.svelte.ts`
//! ALSO resolves against
//! `<workspace>/src/lib/components/ai/loading-labels`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::cache::CacheLayout;

/// Build the overlay tsconfig JSON given the user's tsconfig path and the
/// list of generated `.svelte.ts` files we want type-checked.
///
/// `user_tsconfig` is the *original* user-supplied tsconfig path
/// (absolute), used as the `extends` target. `generated_files` are the
/// absolute paths of the generated `.svelte.ts` files we wrote into the
/// cache.
pub fn build(
    layout: &CacheLayout,
    user_tsconfig: &Path,
    generated_files: &[std::path::PathBuf],
) -> Value {
    // `extends` is resolved relative to the overlay tsconfig dir.
    let extends_rel = relative_from(layout.root.as_path(), user_tsconfig);

    // `files` are absolutized so tsgo doesn't mis-resolve. Always
    // include the svelte type shim so generated files can reference
    // `svelte/*` modules even when the real package isn't installed.
    let mut files: Vec<String> = generated_files
        .iter()
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    files.push(layout.svelte_shims.to_string_lossy().into_owned());

    let mut root_dirs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push_root = |dir: &Path, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
        let s = dir.to_string_lossy().to_string();
        if seen.insert(s.clone()) {
            out.push(s);
        }
    };
    // Overlay svelte subdir first — that's where generated files live.
    push_root(layout.svelte_dir.as_path(), &mut root_dirs, &mut seen);
    // Workspace root second — fallback for projects without their own
    // rootDirs entries.
    push_root(layout.root.as_path().parent().unwrap_or(Path::new("")), &mut root_dirs, &mut seen);
    // Whatever the user's extends chain declared.
    for entry in collect_user_root_dirs(user_tsconfig) {
        push_root(entry.as_path(), &mut root_dirs, &mut seen);
    }

    json!({
        "extends": extends_rel,
        "compilerOptions": {
            "noEmit": true,
            "allowArbitraryExtensions": true,
            "incremental": true,
            "tsBuildInfoFile": layout.tsbuildinfo.to_string_lossy(),
            "skipLibCheck": true,
            // tsgo (the typescript-go preview) has removed the legacy
            // `node`/`node10` moduleResolution values. Many existing
            // tsconfigs still use them. Force `bundler` in the overlay so
            // tsgo accepts the project; the user's runtime moduleResolution
            // setting is unaffected (we never touch their files).
            "moduleResolution": "bundler",
            "module": "esnext",
            "rootDirs": root_dirs,
        },
        "files": files,
    })
}

/// Walk the user tsconfig's `extends` chain (depth-limited; tsconfig
/// chains shouldn't realistically exceed a handful of links) and return
/// every `rootDirs` entry resolved to an absolute path.
///
/// Unreadable, malformed, or missing files cause the walk to stop — we
/// don't care about producing perfect output, only about preserving as
/// much of the user's intent as we can. The overlay still works (just
/// with fewer virtual roots merged) when this returns empty.
fn collect_user_root_dirs(tsconfig: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut current: Option<PathBuf> = Some(tsconfig.to_path_buf());
    let mut hops = 0usize;
    while let Some(path) = current.take() {
        hops += 1;
        if hops > 16 {
            break;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            break;
        };
        let Ok(json) = json5::from_str::<Value>(&content) else {
            break;
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        if let Some(rd) = json
            .get("compilerOptions")
            .and_then(|c| c.get("rootDirs"))
            .and_then(|v| v.as_array())
        {
            for entry in rd {
                if let Some(s) = entry.as_str() {
                    let resolved = if Path::new(s).is_absolute() {
                        PathBuf::from(s)
                    } else {
                        dir.join(s)
                    };
                    out.push(normalize(&resolved));
                }
            }
        }
        // Follow `extends` (string only — we don't support extends arrays
        // yet; rare in practice).
        current = json
            .get("extends")
            .and_then(|v| v.as_str())
            .map(|s| {
                if Path::new(s).is_absolute() {
                    PathBuf::from(s)
                } else {
                    dir.join(s)
                }
            });
    }
    out
}

/// Collapse `..` segments without touching the filesystem. Pure path
/// arithmetic — necessary because `Path::canonicalize` requires the path
/// to exist, and rootDirs entries from extends chains often point at
/// locations that won't exist for every user.
fn normalize(p: &Path) -> PathBuf {
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

/// Compute a relative path from `from` to `to`, falling back to the
/// absolute `to` if a relative path can't be expressed (different roots).
///
/// Used so the overlay's `extends` path is relative when possible — keeps
/// generated tsconfigs portable across machines / CI cache layouts.
fn relative_from(from: &Path, to: &Path) -> String {
    if let Ok(rel) = pathdiff(to, from) {
        return rel.to_string_lossy().into_owned();
    }
    to.to_string_lossy().into_owned()
}

/// Tiny inline path-diff implementation. Returns the path you'd append to
/// `from` to reach `to`, using `..` segments as needed.
///
/// Doesn't follow symlinks or canonicalize; both inputs should already be
/// absolute and in the same logical filesystem.
fn pathdiff(to: &Path, from: &Path) -> Result<std::path::PathBuf, ()> {
    use std::path::{Component, PathBuf};

    let to_components: Vec<_> = to.components().collect();
    let from_components: Vec<_> = from.components().collect();

    if to.has_root() != from.has_root() {
        return Err(());
    }

    let mut common = 0;
    while common < to_components.len()
        && common < from_components.len()
        && to_components[common] == from_components[common]
    {
        common += 1;
    }

    let mut result = PathBuf::new();
    for _ in common..from_components.len() {
        // Each remaining segment in `from` requires a `..`.
        result.push(Component::ParentDir);
    }
    for c in &to_components[common..] {
        result.push(c);
    }

    if result.as_os_str().is_empty() {
        Ok(PathBuf::from("."))
    } else {
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pathdiff_sibling_dirs() {
        let to = PathBuf::from("/a/b/foo.json");
        let from = PathBuf::from("/a/b/.cache");
        let diff = pathdiff(&to, &from).unwrap();
        assert_eq!(diff, PathBuf::from("../foo.json"));
    }

    #[test]
    fn pathdiff_descendant() {
        let to = PathBuf::from("/a/b/c/d.json");
        let from = PathBuf::from("/a/b");
        let diff = pathdiff(&to, &from).unwrap();
        assert_eq!(diff, PathBuf::from("c/d.json"));
    }

    #[test]
    fn pathdiff_same_dir() {
        let to = PathBuf::from("/a/b/x.json");
        let from = PathBuf::from("/a/b");
        let diff = pathdiff(&to, &from).unwrap();
        assert_eq!(diff, PathBuf::from("x.json"));
    }

    #[test]
    fn build_overlay_sets_required_compiler_options() {
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let gen_files = vec![PathBuf::from(
            "/projects/app/.svelte-check/svelte/++Index.svelte.ts",
        )];
        let overlay = build(&layout, &user_ts, &gen_files);

        let opts = &overlay["compilerOptions"];
        assert_eq!(opts["noEmit"], json!(true));
        assert_eq!(opts["allowArbitraryExtensions"], json!(true));
        assert_eq!(opts["incremental"], json!(true));
        assert!(opts["tsBuildInfoFile"].is_string());
    }

    #[test]
    fn build_overlay_extends_user_tsconfig_relatively() {
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let overlay = build(&layout, &user_ts, &[]);
        // extends should point ../tsconfig.json (overlay is in
        // /projects/app/.svelte-check/, user ts in /projects/app/).
        assert_eq!(overlay["extends"], json!("../tsconfig.json"));
    }

    #[test]
    fn build_overlay_lists_generated_files_absolute() {
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let gen_files = vec![
            PathBuf::from("/projects/app/.svelte-check/svelte/++A.svelte.ts"),
            PathBuf::from("/projects/app/.svelte-check/svelte/sub/++B.svelte.ts"),
        ];
        let overlay = build(&layout, &user_ts, &gen_files);
        let files = overlay["files"].as_array().unwrap();
        // 2 generated + 1 svelte-shims.d.ts = 3.
        assert_eq!(files.len(), 3);
        assert!(files[0].as_str().unwrap().ends_with("++A.svelte.ts"));
        assert!(files[1].as_str().unwrap().ends_with("++B.svelte.ts"));
        assert!(files[2].as_str().unwrap().ends_with("svelte-shims.d.ts"));
    }

    #[test]
    fn build_overlay_includes_svelte_shims_when_no_generated_files() {
        // Even with zero `.svelte` files, the shim must still appear so
        // standalone `.ts`/`.js` files in the project can import from
        // svelte/* modules.
        let layout = CacheLayout::for_workspace("/projects/app");
        let user_ts = PathBuf::from("/projects/app/tsconfig.json");
        let overlay = build(&layout, &user_ts, &[]);
        let files = overlay["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().ends_with("svelte-shims.d.ts"));
    }
}
