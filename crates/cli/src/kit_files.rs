//! SvelteKit-file discovery.
//!
//! Mirrors upstream `svelte-check`'s enumeration logic: the COMPLETED
//! line's `<N> FILES` denominator counts every `.svelte` file PLUS
//! every `.ts`/`.js` file that SvelteKit recognizes as a "Kit file"
//! (route, hooks, params). Upstream's pipeline injects `$types`
//! imports into those Kit files, which is why they count as processed
//! entries even though their content is user `.ts`. We don't inject
//! anything, but we enumerate the same set so our denominator matches
//! theirs.
//!
//! Source of truth: `svelte-check@4.4.6`'s `isKitFile` / `isKitRouteFile`
//! / `isHooksFile` / `isParamsFile` in `dist/src/index.js`. Defaults
//! come from the same bundle:
//!
//! ```text
//! paramsPath:         'src/params'
//! serverHooksPath:    'src/hooks.server'
//! clientHooksPath:    'src/hooks.client'
//! universalHooksPath: 'src/hooks'
//! ```
//!
//! Custom overrides in `svelte.config.js`'s `kit.files` are not yet
//! read — defaults cover the overwhelming majority of projects.

use std::path::Path;

/// Resolved SvelteKit file-location settings. Defaults match
/// `defaultKitFilesSettings` in upstream svelte-check.
#[derive(Debug, Clone)]
pub struct KitFilesSettings {
    pub params_path: String,
    pub server_hooks_path: String,
    pub client_hooks_path: String,
    pub universal_hooks_path: String,
}

impl Default for KitFilesSettings {
    fn default() -> Self {
        Self {
            params_path: "src/params".into(),
            server_hooks_path: "src/hooks.server".into(),
            client_hooks_path: "src/hooks.client".into(),
            universal_hooks_path: "src/hooks".into(),
        }
    }
}

/// Return true iff `path` is a SvelteKit file under any of the four
/// recognized categories (route / server-hooks / client-hooks /
/// universal-hooks / params).
///
/// Path is the full absolute/relative path including extension; the
/// function slices basename and extension as upstream does.
pub fn is_kit_file(path: &Path, settings: &KitFilesSettings) -> bool {
    let Some(basename) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    let Some(path_str) = path.to_str() else {
        return false;
    };
    is_kit_route_file(basename)
        || is_hooks_file(path_str, basename, &settings.server_hooks_path)
        || is_hooks_file(path_str, basename, &settings.client_hooks_path)
        || is_hooks_file(path_str, basename, &settings.universal_hooks_path)
        || is_params_file(path_str, basename, &settings.params_path)
}

/// Route files: `+page`, `+layout`, `+page.server`, `+layout.server`,
/// `+server` with any `.ts`/`.js` extension. `@groups` suffix (e.g.
/// `+layout@(auth).svelte`) strips the `@` segment before matching.
fn is_kit_route_file(basename: &str) -> bool {
    let stem = if let Some(at_idx) = basename.find('@') {
        &basename[..at_idx]
    } else {
        match basename.rfind('.') {
            Some(idx) => &basename[..idx],
            None => basename,
        }
    };
    matches!(
        stem,
        "+page" | "+layout" | "+page.server" | "+layout.server" | "+server"
    )
}

/// Hooks files match either:
/// - `src/hooks.server.ts` (full path minus extension ends with hooks path)
/// - `src/hooks.server/index.ts` (dir ends with hooks path + basename is `index.ts|js`)
fn is_hooks_file(path: &str, basename: &str, hooks_path: &str) -> bool {
    if (basename == "index.ts" || basename == "index.js")
        && let Some(dir_end) = path.len().checked_sub(basename.len() + 1)
        && path[..dir_end].ends_with(hooks_path)
    {
        return true;
    }
    // Strip the extension (last `.ext`) and check the path stem.
    let Some(ext_idx) = basename.rfind('.') else {
        return false;
    };
    let ext_len = basename.len() - ext_idx;
    let Some(stem_end) = path.len().checked_sub(ext_len) else {
        return false;
    };
    path[..stem_end].ends_with(hooks_path)
}

/// Params files live under `src/params/` (or configured equivalent)
/// and exclude `.test` / `.spec` variants.
fn is_params_file(path: &str, basename: &str, params_path: &str) -> bool {
    if basename.contains(".test") || basename.contains(".spec") {
        return false;
    }
    let Some(dir_end) = path.len().checked_sub(basename.len() + 1) else {
        return false;
    };
    path[..dir_end].ends_with(params_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn settings() -> KitFilesSettings {
        KitFilesSettings::default()
    }

    #[test]
    fn route_files_are_kit_files() {
        let s = settings();
        assert!(is_kit_file(&p("src/routes/+page.ts"), &s));
        assert!(is_kit_file(&p("src/routes/+page.server.ts"), &s));
        assert!(is_kit_file(&p("src/routes/+layout.ts"), &s));
        assert!(is_kit_file(&p("src/routes/+layout.server.ts"), &s));
        assert!(is_kit_file(&p("src/routes/api/+server.ts"), &s));
        assert!(is_kit_file(&p("src/routes/+page.js"), &s));
    }

    #[test]
    fn route_group_at_suffix_strips_before_matching() {
        // `+layout@(auth).ts` -> stem `+layout` -> route file.
        let s = settings();
        assert!(is_kit_file(&p("src/routes/+layout@(auth).ts"), &s));
        assert!(is_kit_file(&p("src/routes/+page@default.ts"), &s));
    }

    #[test]
    fn non_route_files_are_not_route_kit_files() {
        let s = settings();
        assert!(!is_kit_file(&p("src/lib/foo.ts"), &s));
        assert!(!is_kit_file(&p("src/routes/page.ts"), &s)); // no `+` prefix
        assert!(!is_kit_file(&p("src/routes/+foo.ts"), &s));
        assert!(!is_kit_file(&p("src/routes/+layoutx.ts"), &s));
    }

    #[test]
    fn hooks_files_via_extension_form() {
        let s = settings();
        assert!(is_kit_file(&p("src/hooks.server.ts"), &s));
        assert!(is_kit_file(&p("src/hooks.client.ts"), &s));
        assert!(is_kit_file(&p("src/hooks.ts"), &s));
        assert!(is_kit_file(&p("src/hooks.js"), &s));
    }

    #[test]
    fn hooks_files_via_dir_index_form() {
        // `src/hooks.server/index.ts` is the directory form.
        let s = settings();
        assert!(is_kit_file(&p("src/hooks.server/index.ts"), &s));
        assert!(is_kit_file(&p("src/hooks.client/index.js"), &s));
    }

    #[test]
    fn non_hooks_paths_are_not_hooks_files() {
        let s = settings();
        assert!(!is_kit_file(&p("src/other/hooks.ts"), &s));
        assert!(!is_kit_file(&p("src/hooks-extra.ts"), &s));
    }

    #[test]
    fn params_files_under_params_dir() {
        let s = settings();
        assert!(is_kit_file(&p("src/params/videoId.ts"), &s));
        assert!(is_kit_file(&p("src/params/channelId.js"), &s));
    }

    #[test]
    fn params_excludes_test_and_spec() {
        let s = settings();
        assert!(!is_kit_file(&p("src/params/videoId.test.ts"), &s));
        assert!(!is_kit_file(&p("src/params/videoId.spec.ts"), &s));
    }

    #[test]
    fn custom_hooks_path_override() {
        let s = KitFilesSettings {
            server_hooks_path: "src/server/hooks.server".into(),
            ..KitFilesSettings::default()
        };
        assert!(is_kit_file(&p("src/server/hooks.server.ts"), &s));
        // Default location no longer matches because override doesn't touch
        // the other three paths — but universal `hooks` default IS still set,
        // and `src/hooks.server.ts` stem ends with `src/hooks.server` which
        // does NOT end with `src/hooks`. So it's not a universal-hooks hit.
        assert!(!is_kit_file(&p("src/hooks.server.ts"), &s));
    }
}
