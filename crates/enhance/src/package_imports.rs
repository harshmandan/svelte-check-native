//! `.svelte` files imported through package.json `imports`.
//!
//! SvelteKit 3 replaces the `$lib` path alias with a subpath import
//! (`"imports": { "#lib/*": "./src/lib/*" }`), so components arrive as
//! `import Card from '#lib/Card.svelte'`. The compiler resolves that
//! through package.json to `src/lib/Card.svelte`, where no type file
//! sits: the type file for a component lives in the overlay's mirror of
//! the source tree. `rootDirs` joins the two trees for relative imports
//! only, so the import falls through to svelte's `declare module
//! '*.svelte'` wildcard and every named import from the component is
//! TS2614. The default `svelte-check` engine resolves `.svelte` files
//! itself and reports none of this; `--tsgo` reports all of it.
//!
//! `$lib` already works because the overlay rewrites every `paths`
//! entry to list the source directory followed by its overlay mirror.
//! The aliases here feed the same rewrite: each `imports` entry becomes
//! a `paths` entry, which the compiler consults before package.json.
//! See `design/package_imports/`.

use std::path::{Component, Path, PathBuf};

/// The `imports` entries of the package that owns `tsconfig`, as
/// `paths`-style aliases: the pattern, and the absolute target with its
/// `*` kept. Only entries `paths` can say the same way are returned — a
/// plain string target inside the package, with at most one `*` on each
/// side and as many in the target as in the pattern. Conditional targets
/// (`{ "types": …, "default": … }`) are left to the compiler.
pub fn package_import_aliases(tsconfig: &Path) -> Vec<(String, PathBuf)> {
    let Some(start) = tsconfig.parent() else {
        return Vec::new();
    };
    let Some(manifest) = start
        .ancestors()
        .map(|dir| dir.join("package.json"))
        .find(|p| p.is_file())
    else {
        return Vec::new();
    };
    let Some(pkg_dir) = manifest.parent() else {
        return Vec::new();
    };
    let Some(imports) = std::fs::read_to_string(&manifest)
        .ok()
        .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
        .and_then(|pkg| pkg.get("imports").cloned())
    else {
        return Vec::new();
    };
    let Some(imports) = imports.as_object() else {
        return Vec::new();
    };
    imports
        .iter()
        .filter_map(|(pattern, target)| {
            let target = target.as_str()?;
            let stars = pattern.matches('*').count();
            let relative = target.strip_prefix("./")?;
            let stays_inside = Path::new(relative)
                .components()
                .all(|c| matches!(c, Component::Normal(_) | Component::CurDir));
            (pattern.starts_with('#')
                && stars <= 1
                && target.matches('*').count() == stars
                && stays_inside)
                .then(|| (pattern.clone(), pkg_dir.join(relative)))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn aliases(package_json: &str) -> Vec<(String, PathBuf)> {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("package.json"), package_json).unwrap();
        let app = tmp.path().join("app");
        std::fs::create_dir(&app).unwrap();
        // The tsconfig sits below the package root: the owning package is
        // the nearest package.json above it.
        aliases_relative(&app.join("tsconfig.json"), tmp.path())
    }

    fn aliases_relative(tsconfig: &Path, root: &Path) -> Vec<(String, PathBuf)> {
        package_import_aliases(tsconfig)
            .into_iter()
            .map(|(p, t)| (p, t.strip_prefix(root).unwrap().to_path_buf()))
            .collect()
    }

    #[test]
    fn sveltekit_lib_entries_become_aliases() {
        let got = aliases(
            r##"{ "imports": { "#lib": "./src/lib/index.js", "#lib/*": "./src/lib/*" } }"##,
        );
        assert_eq!(
            got,
            vec![
                ("#lib".to_string(), PathBuf::from("src/lib/index.js")),
                ("#lib/*".to_string(), PathBuf::from("src/lib/*")),
            ]
        );
    }

    #[test]
    fn entries_paths_cannot_express_are_skipped() {
        let got = aliases(
            r##"{ "imports": {
                "#cond/*": { "types": "./types/*", "default": "./src/*" },
                "#pkg": "some-package",
                "#up/*": "./../outside/*",
                "#mismatch/*": "./src/lib/x.js",
                "#ok/*.js": "./src/*.ts"
            } }"##,
        );
        assert_eq!(
            got,
            vec![("#ok/*.js".to_string(), PathBuf::from("src/*.ts"))]
        );
    }

    #[test]
    fn no_imports_field_means_no_aliases() {
        assert!(aliases(r#"{ "name": "x" }"#).is_empty());
    }
}
