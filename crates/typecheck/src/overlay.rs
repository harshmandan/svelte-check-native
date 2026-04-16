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
    let push_root =
        |dir: &Path, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
            let s = dir.to_string_lossy().to_string();
            if seen.insert(s.clone()) {
                out.push(s);
            }
        };
    // Overlay svelte subdir first — that's where generated files live.
    push_root(layout.svelte_dir.as_path(), &mut root_dirs, &mut seen);
    // Workspace root second — fallback for projects without their own
    // rootDirs entries.
    push_root(
        layout.root.as_path().parent().unwrap_or(Path::new("")),
        &mut root_dirs,
        &mut seen,
    );
    // Whatever the user's extends chain declared.
    for entry in collect_user_root_dirs(user_tsconfig) {
        push_root(entry.as_path(), &mut root_dirs, &mut seen);
    }

    // Path aliases: collect every entry the user defined in their
    // extends chain and prepend a cache-mirror candidate. TS resolves
    // each pattern's value-list in order, so a path-mapped import like
    // `$lib/foo/Bar.svelte.ts` first tries our generated overlay file
    // and falls back to the source location if not found. Without this,
    // path-mapped Svelte imports skip rootDirs entirely and never reach
    // our overlay (rootDirs only kicks in for raw relative paths).
    let mut paths_map: serde_json::Map<String, Value> = serde_json::Map::new();
    for (pattern, values) in collect_user_paths(user_tsconfig) {
        let mut merged: Vec<String> = Vec::new();
        for v in &values {
            // The cache mirror sits at <overlay>/svelte/<same-relative-segment>.
            // The tail after the workspace root in `v` is what we mirror.
            let mirrored = mirror_into_overlay(layout, v);
            if let Some(m) = mirrored {
                if !merged.iter().any(|x| x == &m) {
                    merged.push(m);
                }
            }
        }
        for v in values {
            if !merged.iter().any(|x| x == &v) {
                merged.push(v);
            }
        }
        paths_map.insert(
            pattern,
            Value::Array(merged.into_iter().map(Value::String).collect()),
        );
    }

    let mut compiler_options = serde_json::Map::new();
    compiler_options.insert("noEmit".into(), json!(true));
    compiler_options.insert("allowArbitraryExtensions".into(), json!(true));
    // `allowImportingTsExtensions` lets emit rewrite
    // `import './X.svelte'` → `import './X.svelte.ts'` so the import
    // lands on our generated overlay file rather than resolving to the
    // `*.svelte` ambient declaration shipped with the `svelte` package.
    compiler_options.insert("allowImportingTsExtensions".into(), json!(true));
    compiler_options.insert("incremental".into(), json!(true));
    compiler_options.insert(
        "tsBuildInfoFile".into(),
        json!(layout.tsbuildinfo.to_string_lossy()),
    );
    compiler_options.insert("skipLibCheck".into(), json!(true));
    // tsgo has removed the legacy `node`/`node10` moduleResolution
    // values. Force `bundler` in the overlay so tsgo accepts the
    // project; the user's runtime moduleResolution setting is
    // unaffected (we never touch their files).
    compiler_options.insert("moduleResolution".into(), json!("bundler"));
    compiler_options.insert("module".into(), json!("esnext"));
    compiler_options.insert("rootDirs".into(), json!(root_dirs));
    if !paths_map.is_empty() {
        compiler_options.insert("paths".into(), Value::Object(paths_map));
        // tsgo still requires baseUrl when paths is set (despite tsc
        // 5.x removing it as a top-level option). Set it to the cache
        // root — paths-target values are already absolute, so baseUrl
        // is essentially unused for resolution. The TS5102 deprecation
        // warning that tsgo emits is filtered out in
        // svn-typecheck::map_diagnostic.
        compiler_options.insert("baseUrl".into(), json!(layout.root.to_string_lossy()));
    }

    json!({
        "extends": extends_rel,
        "compilerOptions": Value::Object(compiler_options),
        "files": files,
    })
}

/// Map an absolute paths-target path INTO the overlay svelte tree. If
/// the input path is not under the workspace root, return None — the
/// mirror only makes sense for paths inside the project we generated
/// overlays for.
fn mirror_into_overlay(layout: &CacheLayout, path_str: &str) -> Option<String> {
    let p = Path::new(path_str);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        layout.root.as_path().parent()?.join(p)
    };
    let normalized = normalize(&abs);
    let ws_parent = layout.root.as_path().parent()?;
    let rel = normalized.strip_prefix(ws_parent).ok()?;
    let mirrored = layout.svelte_dir.join(rel);
    Some(mirrored.to_string_lossy().into_owned())
}

/// Walk the user tsconfig's `extends` chain and collect every `paths`
/// entry, resolving relative paths-target values against the declaring
/// tsconfig's directory (or its `baseUrl` when set). Inner configs
/// override outer (as TS does).
fn collect_user_paths(tsconfig: &Path) -> Vec<(String, Vec<String>)> {
    use std::collections::HashMap;
    let mut accumulated: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

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
        let opts = json.get("compilerOptions");
        // baseUrl resolves relative paths inside `paths` values.
        let base_url = opts
            .and_then(|c| c.get("baseUrl"))
            .and_then(Value::as_str)
            .map(|b| {
                if Path::new(b).is_absolute() {
                    PathBuf::from(b)
                } else {
                    dir.join(b)
                }
            })
            .unwrap_or_else(|| dir.to_path_buf());
        if let Some(p) = opts.and_then(|c| c.get("paths")).and_then(Value::as_object) {
            for (pattern, values) in p {
                if accumulated.contains_key(pattern) {
                    // Inner (closer-to-root) wins. Skip outer entries.
                    continue;
                }
                let arr = match values.as_array() {
                    Some(a) => a,
                    None => continue,
                };
                let mut resolved: Vec<String> = Vec::new();
                for v in arr {
                    let Some(s) = v.as_str() else { continue };
                    let abs = if Path::new(s).is_absolute() {
                        PathBuf::from(s)
                    } else {
                        base_url.join(s)
                    };
                    resolved.push(normalize(&abs).to_string_lossy().into_owned());
                }
                if !resolved.is_empty() {
                    order.push(pattern.clone());
                    accumulated.insert(pattern.clone(), resolved);
                }
            }
        }
        current = json.get("extends").and_then(|v| v.as_str()).map(|s| {
            if Path::new(s).is_absolute() {
                PathBuf::from(s)
            } else {
                dir.join(s)
            }
        });
    }
    order
        .into_iter()
        .filter_map(|k| accumulated.remove(&k).map(|v| (k, v)))
        .collect()
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
        current = json.get("extends").and_then(|v| v.as_str()).map(|s| {
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
