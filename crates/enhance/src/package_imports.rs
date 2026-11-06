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
//!
//! `paths` is consulted before package.json, so a wildcard alias must
//! never capture a specifier that package.json gives to another key.
//! Both sides pick an exact key over any pattern and otherwise the
//! pattern with the longest text before its `*`, so an overlap is safe
//! when the other key is an alias too and the two prefixes differ in
//! length. Any other overlap leaves the wildcard to the compiler.

use std::path::{Component, Path, PathBuf};

/// The `imports` entries of the package that owns `tsconfig`, as
/// `paths`-style aliases: the pattern, and the absolute target with its
/// `*` kept. Only entries `paths` can say the same way are returned — a
/// plain string target inside the package, with at most one `*` on each
/// side and as many in the target as in the pattern. Conditional targets
/// (`{ "types": …, "default": … }`) are left to the compiler, and so is
/// any entry that can't reach a `.svelte` file. A trailing `*` on both
/// sides is narrowed to `*.svelte`. A wildcard is left to native
/// resolution when it overlaps a key that isn't an alias, or a wildcard
/// alias with a prefix of the same length.
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
    let mut candidates: Vec<(&String, String, PathBuf)> = imports
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
            Some((pattern, alias, pkg_dir.join(relative)))
        })
        .collect();
    // Dropping one wildcard can expose another that it was shielding, so
    // repeat until nothing changes.
    loop {
        let unsafe_keys: Vec<&String> = candidates
            .iter()
            .filter(|(key, alias, _)| {
                let Some((prefix, suffix)) = alias.split_once('*') else {
                    return false;
                };
                imports.keys().any(|other| {
                    other != *key
                        && overlaps(prefix, suffix, other)
                        && !candidates.iter().any(|(k, other_alias, _)| {
                            *k == other
                                && other_alias
                                    .split_once('*')
                                    .is_none_or(|(p, _)| p.len() != prefix.len())
                        })
                })
            })
            .map(|(key, _, _)| *key)
            .collect();
        if unsafe_keys.is_empty() {
            break;
        }
        candidates.retain(|(key, _, _)| !unsafe_keys.contains(key));
    }
    candidates
        .into_iter()
        .map(|(_, alias, target)| (alias, target))
        .collect()
}

/// Whether some specifier matches both the `prefix*suffix` pattern and
/// the imports key `other`.
fn overlaps(prefix: &str, suffix: &str, other: &str) -> bool {
    match other.split_once('*') {
        Some((other_prefix, other_suffix)) => {
            (prefix.starts_with(other_prefix) || other_prefix.starts_with(prefix))
                && (suffix.ends_with(other_suffix) || other_suffix.ends_with(suffix))
        }
        None => other
            .strip_prefix(prefix)
            .is_some_and(|rest| rest.ends_with(suffix)),
    }
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
    fn nested_wildcard_aliases_keep_longest_prefix_precedence() {
        // `paths` and package.json both send `#lib/server/X.svelte` to
        // the longer prefix, so both aliases are safe.
        assert_eq!(
            aliases(
                r##"{ "imports": {
                "#lib/*": "./src/lib/*", "#lib/server/*": "./src/server/*"
            } }"##
            ),
            vec![
                (
                    "#lib/*.svelte".to_string(),
                    PathBuf::from("src/lib/*.svelte")
                ),
                (
                    "#lib/server/*.svelte".to_string(),
                    PathBuf::from("src/server/*.svelte")
                ),
            ]
        );
    }

    #[test]
    fn dropping_a_wildcard_drops_the_wildcards_it_shielded() {
        // The null key rules out `#lib/server/*`; without it as an alias,
        // `#lib/*.svelte` would capture `#lib/server/` specifiers too.
        assert!(
            aliases(
                r##"{ "imports": {
                "#lib/*": "./src/lib/*",
                "#lib/server/*": "./src/server/*",
                "#lib/server/x.svelte": null
            } }"##
            )
            .is_empty()
        );
    }

    #[test]
    fn same_length_prefixes_stay_with_native_resolution() {
        // Precedence between equal prefixes differs between `paths` and
        // package.json, so neither becomes an alias.
        assert!(
            aliases(
                r##"{ "imports": {
                "#lib/*": "./src/lib/*", "#lib/*.svelte": "./src/ui/*.svelte"
            } }"##
            )
            .is_empty()
        );
    }

    #[test]
    fn no_imports_field_means_no_aliases() {
        assert!(aliases(r#"{ "name": "x" }"#).is_empty());
    }
}
