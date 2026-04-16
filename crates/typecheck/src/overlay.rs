//! Generate the overlay tsconfig that tsgo runs against.
//!
//! The overlay extends the user's tsconfig and re-points it at the
//! generated `.svelte.ts` files in the cache. Forces the flags tsgo needs
//! to consume our virtual files (`allowArbitraryExtensions`, `noEmit`,
//! incremental build info location).
//!
//! ### Current scope
//!
//! Minimal: lists generated files explicitly via `files`, sets the
//! mandatory overrides, extends the user tsconfig. Doesn't yet rebase
//! `paths`/`rootDirs`/`include`/`exclude` from the user config — those
//! arrive in a follow-up commit when path-aliased fixtures need them.
//! Pure pass-through extends works for the upstream test fixtures whose
//! tsconfigs use plain `include` patterns.

use std::path::Path;

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
        },
        "files": files,
    })
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
