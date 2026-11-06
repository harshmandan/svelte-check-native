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
//!
//! Only `.svelte` targets become aliases: `#lib/*` is narrowed to
//! `#lib/*.svelte`. `paths` probes extensions and package.json `imports`
//! does not, so a whole-directory alias let `#lib/util` (no extension)
//! resolve here while `tsc`, `svelte-check` and Vite's build reject it.
//! Wildcard aliases that overlap another imports key stay with the
//! compiler too: `paths` cannot preserve native precedence for targets
//! it cannot express. Only `.svelte` targets become aliases.

use std::path::{Component, Path, PathBuf};

/// The `imports` entries of the package that owns `tsconfig`, as
/// `paths`-style aliases: the pattern, and the absolute target with its
/// `*` kept. Only entries `paths` can say the same way are returned — a
/// plain string target inside the package, with at most one `*` on each
/// side and as many in the target as in the pattern. Conditional targets
/// (`{ "types": …, "default": … }`) are left to the compiler, and so is
/// any entry that can't reach a `.svelte` file. A trailing `*` on both
/// sides is narrowed to `*.svelte`. Wildcards overlapping another imports
/// key are left to native resolution, regardless of that key's target.
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
            if !(pattern.starts_with('#')
                && stars <= 1
                && target.matches('*').count() == stars
                && stays_inside)
            {
                return None;
            }
            let (alias, relative) = if target.ends_with(".svelte") {
                (pattern.clone(), relative.to_string())
            } else if stars == 1 && pattern.ends_with('*') && relative.ends_with('*') {
                (format!("{pattern}.svelte"), format!("{relative}.svelte"))
            } else {
                return None;
            };
            // A paths wildcard must not capture a specifier owned by another
            // imports key, including conditional, null and extensionless targets.
            if let Some((prefix, suffix)) = alias.split_once('*') {
                if imports.keys().any(|other| {
                    if other == pattern {
                        return false;
                    }
                    if let Some((other_prefix, other_suffix)) = other.split_once('*') {
                        (prefix.starts_with(other_prefix) || other_prefix.starts_with(prefix))
                            && (suffix.ends_with(other_suffix) || other_suffix.ends_with(suffix))
                    } else {
                        other
                            .strip_prefix(prefix)
                            .is_some_and(|rest| rest.ends_with(suffix))
                    }
                }) {
                    return None;
                }
            }
            Some((alias, pkg_dir.join(relative)))
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
            vec![(
                "#lib/*.svelte".to_string(),
                PathBuf::from("src/lib/*.svelte")
            )]
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
                "#card.svelte": "./card",
                "#ts/*.js": "./src/*.ts",
                "#ui/*": "./src/ui/*.svelte",
                "#app": "./src/App.svelte"
            } }"##,
        );
        assert_eq!(
            got,
            vec![
                ("#ui/*".to_string(), PathBuf::from("src/ui/*.svelte")),
                ("#app".to_string(), PathBuf::from("src/App.svelte")),
            ]
        );
    }

    #[test]
    fn overlapping_imports_stay_with_native_resolution() {
        for other in [
            r##""#lib/Card.svelte": "./src/Card""##,
            r##""#lib/Card.svelte": "./src/Card.ts""##,
            r##""#lib/Card.svelte": null"##,
            r##""#lib/Card.svelte": { "default": "./src/Card.ts" }"##,
            r##""#lib/C*.svelte": "./src/*.ts""##,
            r##""#*": "./src/*.ts""##,
        ] {
            assert!(
                aliases(&format!(
                    r##"{{ "imports": {{ "#lib/*": "./src/lib/*", {other} }} }}"##
                ))
                .is_empty(),
                "{other}"
            );
        }
        // A key outside the narrowed .svelte wildcard does not interfere.
        assert_eq!(
            aliases(
                r##"{ "imports": {
                "#lib/*": "./src/lib/*", "#lib/Card.js": null
            } }"##
            ),
            vec![(
                "#lib/*.svelte".to_string(),
                PathBuf::from("src/lib/*.svelte")
            )]
        );
    }

    #[test]
    fn no_imports_field_means_no_aliases() {
        assert!(aliases(r#"{ "name": "x" }"#).is_empty());
    }
}
