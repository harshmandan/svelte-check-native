//! Cache directory management for generated `.svelte.ts` files.
//!
//! Layout (matches upstream svelte-check):
//!
//! ```text
//! <workspace>/.svelte-check/             (or .svelte-kit/.svelte-check/ for Kit projects)
//!   tsconfig.json                        overlay tsconfig
//!   tsbuildinfo.json                     tsgo incremental build info
//!   svelte/<mirrored relative path>/
//!     ++Foo.svelte.ts                    generated TypeScript for Foo.svelte
//!     Foo.d.svelte.ts                    re-export stub for module resolution
//! ```
//!
//! The `++` prefix prevents collisions with sibling files in the mirrored
//! tree; `.d.svelte.ts` (rather than `.svelte.d.ts`) is required because
//! `moduleResolution: node16+` won't resolve the latter.
//!
//! `write_if_changed` skips disk writes when the new content matches what's
//! already on disk — keeps tsgo's `.tsbuildinfo` happy and avoids touching
//! mtimes pointlessly.

use std::path::{Path, PathBuf};

/// Where the cache lives, plus path conventions.
#[derive(Debug, Clone)]
pub struct CacheLayout {
    /// Workspace root the cache belongs to.
    pub workspace: PathBuf,
    /// Cache root: `<workspace>/.svelte-check/`.
    pub root: PathBuf,
    /// Generated-TS subdir: `<root>/svelte/`.
    pub svelte_dir: PathBuf,
    /// Overlay tsconfig path: `<root>/tsconfig.json`.
    pub overlay_tsconfig: PathBuf,
    /// `.tsbuildinfo` path passed to tsgo.
    pub tsbuildinfo: PathBuf,
    /// Svelte type shims `.d.ts` path: `<root>/svelte-shims.d.ts`.
    /// Re-emitted on every check from `svelte_shims.d.ts` baked into the
    /// typecheck crate. Provides minimal type definitions for `svelte/*`
    /// imports so projects without the real `svelte` package installed
    /// (e.g. the upstream test fixtures) don't fire TS2307.
    pub svelte_shims: PathBuf,
}

impl CacheLayout {
    /// Compute the layout for a workspace. Doesn't create directories.
    pub fn for_workspace(workspace: impl Into<PathBuf>) -> Self {
        let workspace = workspace.into();
        let root = workspace.join(".svelte-check");
        let svelte_dir = root.join("svelte");
        let overlay_tsconfig = root.join("tsconfig.json");
        let tsbuildinfo = root.join("tsbuildinfo.json");
        let svelte_shims = root.join("svelte-shims.d.ts");
        Self {
            workspace,
            root,
            svelte_dir,
            overlay_tsconfig,
            tsbuildinfo,
            svelte_shims,
        }
    }

    /// Map an original `.svelte` source path to its generated `.svelte.ts`
    /// counterpart inside the cache.
    ///
    /// `<workspace>/lib/Foo.svelte` → `<cache>/svelte/lib/Foo.svelte.ts`.
    ///
    /// We DON'T add a prefix (no more `++`) — the file lives in the cache
    /// directory under a mirrored path, so it never collides with the
    /// real `.svelte` source. Keeping the same basename is what lets
    /// `import './Foo.svelte.js'` (rewritten from `import './Foo.svelte'`
    /// in the emit pass) resolve to this file via TS's standard `.js`
    /// → `.ts` lookup, sidestepping the `*.svelte` ambient module
    /// declaration entirely.
    pub fn generated_path(&self, source: &Path) -> PathBuf {
        let rel = source.strip_prefix(&self.workspace).unwrap_or(source);
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let file_stem = rel
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.svelte");
        let renamed = format!("{file_stem}.ts");
        self.svelte_dir.join(parent).join(renamed)
    }

    /// Reverse the [`generated_path`] mapping — given a path inside the
    /// cache, return the corresponding original `.svelte` source path
    /// (or `None` if the input doesn't match the cache layout).
    ///
    /// Used by the diagnostic-mapping pass to translate tsgo's output
    /// filenames back to user-facing source paths.
    pub fn original_from_generated(&self, generated: &Path) -> Option<PathBuf> {
        let rel = generated.strip_prefix(&self.svelte_dir).ok()?;
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let file = rel.file_name().and_then(|s| s.to_str())?;
        // `Foo.svelte.ts` → `Foo.svelte`. Tolerate the legacy `++`
        // prefix and `.d.svelte.ts` shapes too in case a stale cache
        // from an older binary is around.
        let original_name = if let Some(stripped) = file.strip_prefix("++") {
            stripped.strip_suffix(".ts").unwrap_or(stripped).to_string()
        } else if let Some(stem) = file.strip_suffix(".d.svelte.ts") {
            format!("{stem}.svelte")
        } else {
            file.strip_suffix(".ts")?.to_string()
        };
        Some(self.workspace.join(parent).join(original_name))
    }
}

/// Write a file only if `contents` differs from what's currently on disk.
///
/// Returns `Ok(true)` if a write happened, `Ok(false)` if the file was
/// already up to date.
pub fn write_if_changed(path: &Path, contents: &str) -> std::io::Result<bool> {
    if let Ok(existing) = std::fs::read_to_string(path) {
        if existing == contents {
            return Ok(false);
        }
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn layout_paths_are_under_workspace() {
        let layout = CacheLayout::for_workspace("/projects/foo");
        assert_eq!(layout.root, Path::new("/projects/foo/.svelte-check"));
        assert_eq!(
            layout.svelte_dir,
            Path::new("/projects/foo/.svelte-check/svelte")
        );
        assert_eq!(
            layout.overlay_tsconfig,
            Path::new("/projects/foo/.svelte-check/tsconfig.json")
        );
        assert_eq!(
            layout.tsbuildinfo,
            Path::new("/projects/foo/.svelte-check/tsbuildinfo.json")
        );
        assert_eq!(
            layout.svelte_shims,
            Path::new("/projects/foo/.svelte-check/svelte-shims.d.ts")
        );
    }

    #[test]
    fn generated_path_mirrors_source_basename_under_overlay_svelte_dir() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_path = layout.generated_path(Path::new("/p/src/Foo.svelte"));
        assert_eq!(
            gen_path,
            Path::new("/p/.svelte-check/svelte/src/Foo.svelte.ts")
        );
    }

    #[test]
    fn generated_path_for_workspace_root_file() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_path = layout.generated_path(Path::new("/p/Index.svelte"));
        assert_eq!(
            gen_path,
            Path::new("/p/.svelte-check/svelte/Index.svelte.ts")
        );
    }

    #[test]
    fn original_from_generated_inverts_generated_path() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_path = layout.generated_path(Path::new("/p/src/Foo.svelte"));
        let back = layout.original_from_generated(&gen_path).unwrap();
        assert_eq!(back, Path::new("/p/src/Foo.svelte"));
    }

    #[test]
    fn original_from_generated_tolerates_legacy_dts_form() {
        let layout = CacheLayout::for_workspace("/p");
        // Legacy `.d.svelte.ts` shape — emit no longer writes these,
        // but the inverse mapping still recognizes them so a stale
        // cache from an older binary is handled gracefully.
        let dts = Path::new("/p/.svelte-check/svelte/src/Foo.d.svelte.ts");
        let back = layout.original_from_generated(dts).unwrap();
        assert_eq!(back, Path::new("/p/src/Foo.svelte"));
    }

    #[test]
    fn original_from_generated_returns_none_for_non_cache_paths() {
        let layout = CacheLayout::for_workspace("/p");
        let outside = Path::new("/p/src/Foo.svelte");
        assert!(layout.original_from_generated(outside).is_none());
    }

    #[test]
    fn write_if_changed_creates_parent_dirs() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("a/b/c/file.txt");
        let wrote = write_if_changed(&path, "hello").unwrap();
        assert!(wrote);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "hello");
    }

    #[test]
    fn write_if_changed_skips_when_content_unchanged() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("file.txt");
        write_if_changed(&path, "v1").unwrap();
        let wrote_again = write_if_changed(&path, "v1").unwrap();
        assert!(!wrote_again);
    }

    #[test]
    fn write_if_changed_writes_when_content_changes() {
        let tmp = tempdir().unwrap();
        let path = tmp.path().join("file.txt");
        write_if_changed(&path, "v1").unwrap();
        let wrote_again = write_if_changed(&path, "v2").unwrap();
        assert!(wrote_again);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "v2");
    }
}
