//! Cache directory management for generated `.svelte.ts` files.
//!
//! Layout:
//!
//! ```text
//! <cache root>/
//!   tsconfig.json                        overlay tsconfig
//!   tsbuildinfo.json                     tsgo incremental build info
//!   svelte/<mirrored relative path>/
//!     Foo.svelte.ts                      generated TypeScript for Foo.svelte
//! ```
//!
//! The cache root is chosen by [`CacheLayout::for_workspace`]:
//!
//!   1. `<workspace>/node_modules/.cache/svelte-check-native/` — preferred
//!      when `node_modules/` exists. This directory is gitignored
//!      everywhere (it's the convention used by eslint, prettier,
//!      vite, vitest, ts-loader, etc.) so the cache is invisible to
//!      git without the user having to add anything to `.gitignore`.
//!
//!   2. `<workspace>/.svelte-check/` — fallback when there is no
//!      `node_modules/` (rare; mostly fresh-clone or no-deps test
//!      fixtures). Users who hit this path are expected to add
//!      `.svelte-check/` to their `.gitignore` themselves.
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
    /// When the CLI escaped a solution-style root tsconfig to a
    /// sub-project, this holds the path to the solution root's
    /// `tsconfig.json`. `None` for the common flat-project case.
    /// Consumed by the overlay builder to flatten sibling-project
    /// references into the overlay's include/exclude/paths (see
    /// `svn_core::tsconfig::flatten_references`).
    pub solution_root_tsconfig: Option<PathBuf>,
    /// Cache root: usually `<workspace>/node_modules/.cache/svelte-check-native/`
    /// (gitignored by convention), with `<workspace>/.svelte-check/` as a
    /// fallback when there is no `node_modules`. See [`Self::for_workspace`].
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
    ///
    /// Picks `<workspace>/node_modules/.cache/svelte-check-native/` when
    /// `node_modules/` already exists (the common case in any real
    /// Svelte project), and falls back to `<workspace>/.svelte-check/`
    /// otherwise — see the module docs for why.
    pub fn for_workspace(workspace: impl Into<PathBuf>) -> Self {
        Self::for_workspace_with_solution_root(workspace, None)
    }

    /// Like [`Self::for_workspace`] but records the solution-style
    /// root tsconfig path when the CLI's `escape_solution_tsconfig`
    /// step redirected to a sub-project. The overlay builder reads
    /// it to merge sibling-project `include`/`exclude`/`paths` into
    /// the overlay so transitive imports into referenced projects
    /// don't fire "File not listed within project".
    pub fn for_workspace_with_solution_root(
        workspace: impl Into<PathBuf>,
        solution_root_tsconfig: Option<PathBuf>,
    ) -> Self {
        let workspace = workspace.into();
        let node_modules = workspace.join("node_modules");
        let root = if node_modules.is_dir() {
            node_modules.join(".cache").join("svelte-check-native")
        } else {
            workspace.join(".svelte-check")
        };
        let svelte_dir = root.join("svelte");
        let overlay_tsconfig = root.join("tsconfig.json");
        let tsbuildinfo = root.join("tsbuildinfo.json");
        let svelte_shims = root.join("svelte-shims.d.ts");
        Self {
            workspace,
            solution_root_tsconfig,
            root,
            svelte_dir,
            overlay_tsconfig,
            tsbuildinfo,
            svelte_shims,
        }
    }

    /// Map an original `.svelte` source path to its generated overlay
    /// inside the cache (the file that holds the actual emitted TS).
    ///
    /// `<workspace>/lib/Foo.svelte` → `<cache>/svelte/lib/Foo.svelte.svn.ts`.
    ///
    /// The `.svn.ts` middle segment is what keeps this overlay from
    /// colliding with a user's same-named `.svelte.ts` runes module
    /// (Svelte 5 convention: `Foo.svelte` paired with `Foo.svelte.ts`
    /// for shared runes logic — widely used across the Svelte UI
    /// component-library ecosystem). A plain `.svelte.ts` overlay would live at the exact
    /// virtual path as the user's runes module; with `rootDirs`
    /// cache-first, the overlay shadowed the runes module and
    /// consumers writing `import { useFoo } from './Foo.svelte.js'`
    /// lost the named exports.
    ///
    /// Consumer resolution:
    ///   - `import Foo from './Foo.svelte'` in our own emitted overlay
    ///     code: the emit pass rewrites the specifier to
    ///     `./Foo.svelte.svn.ts`, so it lands here directly.
    ///   - `import Foo from './Foo.svelte'` in USER-controlled files
    ///     (barrel re-exports, `.ts` modules, etc.) that we don't
    ///     touch: TS's `allowArbitraryExtensions` looks for
    ///     `Foo.d.svelte.ts` ambient — see `ambient_path` below,
    ///     which re-exports from this overlay.
    pub fn generated_path(&self, source: &Path) -> PathBuf {
        self.generated_path_with_lang(source, true)
    }

    /// Like [`generated_path`] but lets the caller pick the overlay's
    /// extension based on the source's effective script language.
    /// `is_ts = true` → `.svelte.svn.ts`; `is_ts = false` → `.svelte.svn.js`.
    ///
    /// Mirroring upstream svelte-check's incremental.ts: the overlay
    /// extension is what tells tsgo whether to apply TS-strict
    /// inference (empty array → `never[]`, null literal → `null`) or
    /// JS-loose inference (both → `any` under `noImplicitAny: false`).
    /// On JS-Svelte sources the user's tsconfig usually carries
    /// `noImplicitAny: false`, and emitting a `.ts` overlay forces
    /// strict inference that the user never opted into.
    pub fn generated_path_with_lang(&self, source: &Path, is_ts: bool) -> PathBuf {
        let rel = source.strip_prefix(&self.workspace).unwrap_or(source);
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let file_stem = rel
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.svelte");
        let ext = if is_ts { "ts" } else { "js" };
        let renamed = format!("{file_stem}.svn.{ext}");
        self.svelte_dir.join(parent).join(renamed)
    }

    /// Mirror path for a Kit `.ts` source (route modules, hooks,
    /// params). `<workspace>/src/routes/+server.ts` →
    /// `<cache>/svelte/src/routes/+server.ts`. Same basename, same
    /// extension — tsgo loads this via the overlay tsconfig's
    /// `files` list, and the original source path is added to
    /// `exclude` so tsgo doesn't pick up BOTH the original (untyped
    /// handler destructures) and the overlay (with injected
    /// RequestEvent / Load types) and produce duplicate-declaration
    /// noise.
    pub fn kit_overlay_path(&self, source: &Path) -> PathBuf {
        let rel = source.strip_prefix(&self.workspace).unwrap_or(source);
        self.svelte_dir.join(rel)
    }

    /// Cache mirror of the user's `.svelte-kit/types/` tree:
    /// `<root>/svelte-kit/types/`. Per-route `$types.d.ts` files are
    /// written here with their `'../(…/)src/routes/…/+page.js'`
    /// import chains rewritten to `'../(…/)svelte/src/routes/…/+page.js'`
    /// so the chain lands in our typed Kit-file copy under
    /// [`Self::svelte_dir`] instead of the user's untyped source.
    ///
    /// Wins over the real user-tree `.svelte-kit/types/` via the
    /// overlay tsconfig's `rootDirs` priority (this dir listed FIRST).
    /// Closes the implicit-any cascade at every `data: PageData`
    /// consumer site that would otherwise resolve through the user
    /// `+page.ts` and get widened to `any`.
    pub fn kit_types_mirror_dir(&self) -> PathBuf {
        self.root.join("svelte-kit").join("types")
    }

    /// Ambient-declaration path for a `.svelte` source, sibling to the
    /// overlay. `<workspace>/lib/Foo.svelte` →
    /// `<cache>/svelte/lib/Foo.d.svelte.ts`.
    ///
    /// The `.d.svelte.ts` shape is what TypeScript's
    /// `allowArbitraryExtensions` looks for when resolving
    /// `import './Foo.svelte'` as a non-standard extension. The file
    /// just re-exports from the real overlay
    /// (`export { default } from './Foo.svelte.svn.ts'; export *
    /// from './Foo.svelte.svn.ts';`) so user-controlled modules that
    /// reference `./Foo.svelte` pick up the overlay's types without
    /// our emit having to rewrite their specifiers.
    pub fn ambient_path(&self, source: &Path) -> PathBuf {
        let rel = source.strip_prefix(&self.workspace).unwrap_or(source);
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let file_stem = rel
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("unknown.svelte");
        // Strip the `.svelte` suffix then add `.d.svelte.ts`.
        let bare = file_stem.strip_suffix(".svelte").unwrap_or(file_stem);
        self.svelte_dir
            .join(parent)
            .join(format!("{bare}.d.svelte.ts"))
    }

    /// Reverse the [`generated_path`] / [`ambient_path`] mapping —
    /// given a path inside the cache, return the corresponding
    /// original `.svelte` source path (or `None` if the input doesn't
    /// match the cache layout).
    ///
    /// Used by the diagnostic-mapping pass to translate tsgo's output
    /// filenames back to user-facing source paths. Handles the current
    /// `.svelte.svn.ts` / `.d.svelte.ts` pair plus legacy
    /// `.svelte.ts` and `++` shapes so a stale cache from an older
    /// binary is tolerated.
    pub fn original_from_generated(&self, generated: &Path) -> Option<PathBuf> {
        let rel = generated.strip_prefix(&self.svelte_dir).ok()?;
        let parent = rel.parent().unwrap_or_else(|| Path::new(""));
        let file = rel.file_name().and_then(|s| s.to_str())?;
        let original_name = if let Some(stem) = file.strip_suffix(".svelte.svn.ts") {
            format!("{stem}.svelte")
        } else if let Some(stem) = file.strip_suffix(".svelte.svn.js") {
            // JS-overlay form (emitted when the source has no
            // `<script lang="ts">`). Same reverse map as the TS form —
            // the source basename is the prefix before `.svelte.svn.js`.
            format!("{stem}.svelte")
        } else if let Some(stem) = file.strip_suffix(".d.svelte.ts") {
            format!("{stem}.svelte")
        } else {
            // Kit mirror files (`+page.ts`, `hooks.server.ts`,
            // `src/params/foo.ts`) live at the same basename in cache
            // as in source — see `kit_overlay_path`. The inverse is
            // identity: no extension rewrite.
            file.to_string()
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
    fn layout_paths_are_under_workspace_when_no_node_modules() {
        // `/projects/foo` doesn't exist on disk, so `node_modules.is_dir()`
        // returns false and the fallback `.svelte-check` root kicks in.
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
    fn layout_picks_node_modules_cache_when_present() {
        let tmp = tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules")).unwrap();
        let layout = CacheLayout::for_workspace(tmp.path().to_path_buf());
        let expected_root = tmp
            .path()
            .join("node_modules")
            .join(".cache")
            .join("svelte-check-native");
        assert_eq!(layout.root, expected_root);
        assert_eq!(layout.svelte_dir, expected_root.join("svelte"));
        assert_eq!(layout.overlay_tsconfig, expected_root.join("tsconfig.json"));
    }

    #[test]
    fn layout_falls_back_to_dot_svelte_check_when_no_node_modules() {
        let tmp = tempdir().unwrap();
        // No node_modules created.
        let layout = CacheLayout::for_workspace(tmp.path().to_path_buf());
        assert_eq!(layout.root, tmp.path().join(".svelte-check"));
    }

    #[test]
    fn generated_path_mirrors_source_basename_under_overlay_svelte_dir() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_path = layout.generated_path(Path::new("/p/src/Foo.svelte"));
        assert_eq!(
            gen_path,
            Path::new("/p/.svelte-check/svelte/src/Foo.svelte.svn.ts")
        );
    }

    #[test]
    fn generated_path_for_workspace_root_file() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_path = layout.generated_path(Path::new("/p/Index.svelte"));
        assert_eq!(
            gen_path,
            Path::new("/p/.svelte-check/svelte/Index.svelte.svn.ts")
        );
    }

    #[test]
    fn generated_path_with_lang_emits_js_extension_for_js_sources() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_ts = layout.generated_path_with_lang(Path::new("/p/src/Foo.svelte"), true);
        assert_eq!(
            gen_ts,
            Path::new("/p/.svelte-check/svelte/src/Foo.svelte.svn.ts")
        );
        let gen_js = layout.generated_path_with_lang(Path::new("/p/src/Foo.svelte"), false);
        assert_eq!(
            gen_js,
            Path::new("/p/.svelte-check/svelte/src/Foo.svelte.svn.js")
        );
    }

    #[test]
    fn original_from_generated_inverts_js_overlay_path() {
        let layout = CacheLayout::for_workspace("/p");
        let gen_js = layout.generated_path_with_lang(Path::new("/p/src/Foo.svelte"), false);
        let back = layout.original_from_generated(&gen_js).unwrap();
        assert_eq!(back, Path::new("/p/src/Foo.svelte"));
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
    fn original_from_generated_inverts_kit_overlay_path() {
        let layout = CacheLayout::for_workspace("/p");
        let kit = Path::new("/p/.svelte-check/svelte/src/routes/+page.ts");
        let back = layout.original_from_generated(kit).unwrap();
        assert_eq!(back, Path::new("/p/src/routes/+page.ts"));
    }

    #[test]
    fn original_from_generated_preserves_kit_hooks_extension() {
        let layout = CacheLayout::for_workspace("/p");
        let kit = Path::new("/p/.svelte-check/svelte/src/hooks.server.ts");
        let back = layout.original_from_generated(kit).unwrap();
        assert_eq!(back, Path::new("/p/src/hooks.server.ts"));
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
