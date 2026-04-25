//! Load a tsconfig with full `extends` chain resolution + `${configDir}`
//! substitution + merging.
//!
//! This is the canonical entry point every consumer should use. Given a path
//! to `tsconfig.json`, return a fully-merged [`TsConfigFile`] as if the user
//! had written one giant flat config.
//!
//! ### Resolution
//!
//! - Relative `extends` (`./`, `../`, or absolute path): resolved against the
//!   directory of the config that wrote it. If the path has no extension,
//!   tries `.json` then the bare path.
//! - Package `extends` (e.g. `@tsconfig/svelte`, `@tsconfig/svelte/tsconfig.json`,
//!   `my-tsconfig`): node-style walk up from the current config's dir looking
//!   for `node_modules/<pkg>`. For bare package names (no subpath), honors the
//!   package.json `"tsconfig"` field if present, else defaults to
//!   `tsconfig.json`.
//!
//! ### `${configDir}` substitution
//!
//! Done per-file, before merging. The placeholder expands to the absolute path
//! of the directory containing the file that literally wrote it. So if
//! `base.json` has `"baseUrl": "${configDir}/src"` and the user's
//! `tsconfig.json` extends it, `${configDir}` resolves to *base.json's* dir.
//!
//! ### Merge rules (match TypeScript's behavior)
//!
//! - `compilerOptions`: shallow merge — child's keys override parent's;
//!   parent's keys absent in child are inherited. `raw` is also shallow-merged
//!   so unknown fields inherit the same way.
//! - `paths`: REPLACED entirely if child specifies it (not per-key merge).
//! - `rootDirs`: REPLACED if child specifies non-empty.
//! - `typeRoots` / `types`: REPLACED if child specifies (even empty).
//! - `include`, `exclude`, `files`, `references`: REPLACED if child specifies.
//! - Final config's `path` is set to the entry file (the leaf of the chain).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::{Map, Value};

use super::parse::{ParseError, parse_file};
use super::{CompilerOptions, TsConfigFile};

/// Errors when loading a tsconfig chain.
#[derive(Debug, thiserror::Error)]
pub enum LoadError {
    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error("failed to read {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },

    #[error("extends chain cycle detected at {path}")]
    Cycle { path: PathBuf },

    #[error(
        "could not resolve extends target `{reference}` from {from} \
         (tried relative path and node_modules walk-up)"
    )]
    ExtendsNotFound { reference: String, from: PathBuf },
}

/// Load and fully resolve a tsconfig, following the extends chain.
pub fn load(entry: impl AsRef<Path>) -> Result<TsConfigFile, LoadError> {
    let mut seen = HashSet::new();
    load_recursive(entry.as_ref(), &mut seen)
}

/// Walk the extends chain and return every parsed + `${configDir}`-
/// substituted [`TsConfigFile`] visited, BFS order starting at the
/// entry file. Extends references are resolved with the same rules as
/// [`load`] — relative paths with `.json` inference, package extends via
/// `node_modules` walk-up, array-extends left-to-right.
///
/// Unlike [`load`], this returns each file unmerged. Callers that need
/// custom aggregation across the chain (e.g. the overlay builder, which
/// wants a UNION of `rootDirs` from every config rather than TS's
/// replace-on-child semantics) can iterate the list directly.
///
/// Cycles and unreadable files are skipped silently — the function is
/// best-effort, matching what the overlay builder's hand-rolled walk
/// used to do. Hard errors (missing entry file, malformed entry) still
/// surface via [`LoadError`].
pub fn load_chain(entry: impl AsRef<Path>) -> Result<Vec<TsConfigFile>, LoadError> {
    use std::collections::VecDeque;

    let entry_canon = dunce::canonicalize(entry.as_ref()).map_err(|source| LoadError::Io {
        path: entry.as_ref().to_path_buf(),
        source,
    })?;

    let mut out: Vec<TsConfigFile> = Vec::new();
    let mut visited: HashSet<PathBuf> = HashSet::new();
    let mut queue: VecDeque<PathBuf> = VecDeque::from([entry_canon]);

    while let Some(path) = queue.pop_front() {
        if !visited.insert(path.clone()) {
            continue;
        }
        let mut file = match parse_file(&path) {
            Ok(f) => f,
            Err(_) => continue,
        };
        file.path = path.clone();
        substitute_config_dir(&mut file);

        let parent_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        for ext_ref in &file.extends {
            match resolve_extends(ext_ref, &parent_dir) {
                Ok(resolved) => {
                    let canon = dunce::canonicalize(&resolved).unwrap_or(resolved);
                    if !visited.contains(&canon) {
                        queue.push_back(canon);
                    }
                }
                Err(_) => continue,
            }
        }
        out.push(file);
    }
    Ok(out)
}

fn load_recursive(path: &Path, seen: &mut HashSet<PathBuf>) -> Result<TsConfigFile, LoadError> {
    let canonical = dunce::canonicalize(path).map_err(|source| LoadError::Io {
        path: path.to_path_buf(),
        source,
    })?;

    if !seen.insert(canonical.clone()) {
        return Err(LoadError::Cycle { path: canonical });
    }

    let mut file = parse_file(&canonical)?;
    // parse_file stored the uncanonicalized path; overwrite with the canonical
    // one so $configDir resolves correctly even if the user passed a relative
    // entry path.
    file.path = canonical.clone();
    substitute_config_dir(&mut file);

    let parent_dir = canonical
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    let extends_refs = std::mem::take(&mut file.extends);

    let mut merged = TsConfigFile {
        path: canonical.clone(),
        ..TsConfigFile::default()
    };

    for ext_ref in &extends_refs {
        let resolved = resolve_extends(ext_ref, &parent_dir)?;
        let parent = load_recursive(&resolved, seen)?;
        merge_into(&mut merged, parent);
    }
    merge_into(&mut merged, file);

    // Final path stays at the entry file.
    merged.path = canonical.clone();

    seen.remove(&canonical);
    Ok(merged)
}

// ===== ${configDir} substitution =========================================

fn substitute_config_dir(file: &mut TsConfigFile) {
    let dir = file.config_dir().to_string_lossy().into_owned();

    let sub = |s: &mut String| {
        if s.contains("${configDir}") {
            *s = s.replace("${configDir}", &dir);
        }
    };
    let sub_opt = |s: &mut Option<String>| {
        if let Some(s) = s {
            sub(s);
        }
    };
    let sub_vec = |v: &mut Vec<String>| v.iter_mut().for_each(&sub);
    let sub_opt_vec = |v: &mut Option<Vec<String>>| {
        if let Some(v) = v {
            sub_vec(v);
        }
    };

    let co = &mut file.compiler_options;
    sub_opt(&mut co.base_url);
    sub_vec(&mut co.root_dirs);
    for vs in co.paths.values_mut() {
        sub_vec(vs);
    }
    sub_opt(&mut co.declaration_dir);
    sub_opt_vec(&mut co.type_roots);
    sub_opt_vec(&mut co.types);

    // Walk unknown compilerOptions values too — users can put ${configDir} in
    // anything and we have to pass it through correctly.
    walk_raw(&mut co.raw, &dir);

    sub_opt_vec(&mut file.include);
    sub_opt_vec(&mut file.exclude);
    sub_opt_vec(&mut file.files);
    for r in &mut file.references {
        sub(&mut r.path);
    }
}

fn walk_raw(map: &mut Map<String, Value>, dir: &str) {
    for v in map.values_mut() {
        walk_value(v, dir);
    }
}

fn walk_value(v: &mut Value, dir: &str) {
    match v {
        Value::String(s) => {
            if s.contains("${configDir}") {
                *s = s.replace("${configDir}", dir);
            }
        }
        Value::Array(arr) => {
            for x in arr {
                walk_value(x, dir);
            }
        }
        Value::Object(obj) => {
            for x in obj.values_mut() {
                walk_value(x, dir);
            }
        }
        _ => {}
    }
}

// ===== Extends resolution ================================================

fn resolve_extends(reference: &str, config_dir: &Path) -> Result<PathBuf, LoadError> {
    if is_relative_reference(reference) || Path::new(reference).is_absolute() {
        resolve_relative_extends(reference, config_dir)
    } else {
        resolve_package_extends(reference, config_dir)
    }
}

fn is_relative_reference(s: &str) -> bool {
    s.starts_with("./") || s.starts_with("../") || s.starts_with(".\\") || s.starts_with("..\\")
}

fn resolve_relative_extends(reference: &str, config_dir: &Path) -> Result<PathBuf, LoadError> {
    let candidate = if Path::new(reference).is_absolute() {
        PathBuf::from(reference)
    } else {
        config_dir.join(reference)
    };

    // If there's no extension, try `.json` first.
    if candidate.extension().is_none() {
        let with_json = candidate.with_extension("json");
        if with_json.is_file() {
            return Ok(with_json);
        }
    }

    if candidate.is_file() {
        return Ok(candidate);
    }

    Err(LoadError::ExtendsNotFound {
        reference: reference.to_string(),
        from: config_dir.to_path_buf(),
    })
}

fn resolve_package_extends(reference: &str, start_dir: &Path) -> Result<PathBuf, LoadError> {
    let (pkg, subpath) = split_package_and_subpath(reference);

    let mut cur: Option<&Path> = Some(start_dir);
    while let Some(dir) = cur {
        let pkg_root = dir.join(crate::NODE_MODULES_DIR).join(pkg);
        if pkg_root.is_dir() {
            let resolved = if let Some(sp) = subpath {
                pkg_root.join(sp)
            } else {
                // Prefer package.json's "tsconfig" field; else fall back to
                // tsconfig.json at the package root.
                let pkg_json_path = pkg_root.join("package.json");
                if let Ok(contents) = std::fs::read_to_string(&pkg_json_path) {
                    if let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&contents) {
                        if let Some(Value::String(ts)) = obj.get("tsconfig") {
                            pkg_root.join(ts)
                        } else {
                            pkg_root.join("tsconfig.json")
                        }
                    } else {
                        pkg_root.join("tsconfig.json")
                    }
                } else {
                    pkg_root.join("tsconfig.json")
                }
            };
            if resolved.is_file() {
                return Ok(resolved);
            }
            // Fall through to keep walking up — a closer ancestor might have
            // a node_modules directory but not this package; an outer one
            // might.
        }
        cur = dir.parent();
    }

    Err(LoadError::ExtendsNotFound {
        reference: reference.to_string(),
        from: start_dir.to_path_buf(),
    })
}

/// Split a package-style extends reference into (package-name, subpath).
///
/// - `"my-pkg"` → `("my-pkg", None)`
/// - `"my-pkg/tsconfig.json"` → `("my-pkg", Some("tsconfig.json"))`
/// - `"@scope/pkg"` → `("@scope/pkg", None)`
/// - `"@scope/pkg/a/b.json"` → `("@scope/pkg", Some("a/b.json"))`
fn split_package_and_subpath(reference: &str) -> (&str, Option<&str>) {
    if let Some(scoped) = reference.strip_prefix('@') {
        // Scoped: first `/` ends the scope; second `/` (if any) ends the pkg.
        let Some(first_slash) = scoped.find('/') else {
            return (reference, None);
        };
        let after_scope = &scoped[first_slash + 1..];
        let pkg_end_in_full =
            1 + first_slash + 1 + after_scope.find('/').unwrap_or(after_scope.len());
        if pkg_end_in_full >= reference.len() {
            (reference, None)
        } else {
            (
                &reference[..pkg_end_in_full],
                Some(&reference[pkg_end_in_full + 1..]),
            )
        }
    } else if let Some(slash) = reference.find('/') {
        (&reference[..slash], Some(&reference[slash + 1..]))
    } else {
        (reference, None)
    }
}

// ===== Merge ============================================================

fn merge_into(base: &mut TsConfigFile, child: TsConfigFile) {
    let co = &mut base.compiler_options;
    let cc = child.compiler_options;
    merge_compiler_options(co, cc);

    if child.include.is_some() {
        base.include = child.include;
    }
    if child.exclude.is_some() {
        base.exclude = child.exclude;
    }
    if child.files.is_some() {
        base.files = child.files;
    }
    if !child.references.is_empty() {
        base.references = child.references;
    }
}

fn merge_compiler_options(co: &mut CompilerOptions, cc: CompilerOptions) {
    macro_rules! inherit_opt {
        ($($field:ident),* $(,)?) => {
            $( if cc.$field.is_some() { co.$field = cc.$field; } )*
        };
    }
    inherit_opt!(
        base_url,
        allow_js,
        check_js,
        no_unused_locals,
        no_unused_parameters,
        strict,
        strict_null_checks,
        strict_function_types,
        strict_bind_call_apply,
        no_implicit_any,
        no_implicit_this,
        always_strict,
        module_resolution,
        module,
        target,
        jsx,
        jsx_import_source,
        type_roots,
        types,
        composite,
        declaration,
        declaration_map,
        declaration_dir,
        allow_arbitrary_extensions,
        skip_lib_check,
        verbatim_module_syntax,
        isolated_modules,
        resolve_json_module,
        allow_synthetic_default_imports,
        es_module_interop,
    );

    if !cc.root_dirs.is_empty() {
        co.root_dirs = cc.root_dirs;
    }
    if !cc.paths.is_empty() {
        co.paths = cc.paths;
    }

    // raw: shallow merge (child keys replace parent keys).
    for (k, v) in cc.raw {
        co.raw.insert(k, v);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn write(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn load_without_extends_is_identity() {
        let tmp = tempdir().unwrap();
        let ts = tmp.path().join("tsconfig.json");
        write(
            &ts,
            r#"{ "compilerOptions": { "strict": true, "target": "ES2022" } }"#,
        );

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
        assert!(cfg.extends.is_empty());
    }

    #[test]
    fn load_with_single_relative_extends_merges_fields() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(
            &base,
            r#"{ "compilerOptions": { "strict": true, "target": "ES2020" } }"#,
        );
        write(
            &ts,
            r#"{
                "extends": "./base.json",
                "compilerOptions": { "target": "ES2022" }
            }"#,
        );

        let cfg = load(&ts).unwrap();
        // Target is overridden by child.
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
        // Strict is inherited from base.
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    #[test]
    fn load_with_extension_inferred() {
        // extends: "./base" (no .json suffix) should find base.json.
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "compilerOptions": { "strict": true } }"#);
        write(&ts, r#"{ "extends": "./base" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    #[test]
    fn load_with_array_extends_last_wins_for_conflicts() {
        let tmp = tempdir().unwrap();
        let a = tmp.path().join("a.json");
        let b = tmp.path().join("b.json");
        let ts = tmp.path().join("tsconfig.json");
        write(
            &a,
            r#"{ "compilerOptions": { "target": "ES2018", "strict": true } }"#,
        );
        write(&b, r#"{ "compilerOptions": { "target": "ES2022" } }"#);
        write(&ts, r#"{ "extends": ["./a.json", "./b.json"] }"#);

        let cfg = load(&ts).unwrap();
        // b wins on target.
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
        // a's strict is inherited (b didn't override).
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    #[test]
    fn load_detects_extends_cycle() {
        let tmp = tempdir().unwrap();
        let a = tmp.path().join("a.json");
        let b = tmp.path().join("b.json");
        write(&a, r#"{ "extends": "./b.json" }"#);
        write(&b, r#"{ "extends": "./a.json" }"#);

        let err = load(&a).unwrap_err();
        assert!(matches!(err, LoadError::Cycle { .. }), "got {err:?}");
    }

    #[test]
    fn load_errors_on_missing_extends() {
        let tmp = tempdir().unwrap();
        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "./nonexistent.json" }"#);

        let err = load(&ts).unwrap_err();
        assert!(
            matches!(err, LoadError::ExtendsNotFound { .. }),
            "got {err:?}"
        );
    }

    #[test]
    fn config_dir_substitution_in_child() {
        let tmp = tempdir().unwrap();
        let subdir = tmp.path().join("project");
        fs::create_dir_all(&subdir).unwrap();
        let ts = subdir.join("tsconfig.json");
        write(
            &ts,
            r#"{ "compilerOptions": { "baseUrl": "${configDir}/src" } }"#,
        );

        let cfg = load(&ts).unwrap();
        let expected = subdir.canonicalize().unwrap().join("src");
        assert_eq!(
            cfg.compiler_options.base_url.as_deref(),
            Some(expected.to_str().unwrap())
        );
    }

    #[test]
    fn config_dir_substitution_in_parent_uses_parent_dir() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join("configs");
        let child_dir = tmp.path().join("project");
        fs::create_dir_all(&base_dir).unwrap();
        fs::create_dir_all(&child_dir).unwrap();

        let base = base_dir.join("base.json");
        let ts = child_dir.join("tsconfig.json");

        // Parent's ${configDir} should resolve to the PARENT's dir, not the
        // child's.
        write(
            &base,
            r#"{ "compilerOptions": { "rootDirs": ["${configDir}/src"] } }"#,
        );
        write(&ts, r#"{ "extends": "../configs/base.json" }"#);

        let cfg = load(&ts).unwrap();
        let expected = base_dir.canonicalize().unwrap().join("src");
        assert_eq!(
            cfg.compiler_options.root_dirs,
            vec![expected.to_str().unwrap()]
        );
    }

    #[test]
    fn child_paths_replace_parent_paths_entirely() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(
            &base,
            r#"{
                "compilerOptions": {
                    "paths": { "foo/*": ["./foo/*"], "bar/*": ["./bar/*"] }
                }
            }"#,
        );
        write(
            &ts,
            r#"{
                "extends": "./base.json",
                "compilerOptions": {
                    "paths": { "baz/*": ["./baz/*"] }
                }
            }"#,
        );

        let cfg = load(&ts).unwrap();
        // Child's paths replaced parent's entirely.
        assert_eq!(cfg.compiler_options.paths.len(), 1);
        assert!(cfg.compiler_options.paths.contains_key("baz/*"));
    }

    #[test]
    fn child_include_replaces_parent_include() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "include": ["base/**/*"] }"#);
        write(
            &ts,
            r#"{ "extends": "./base.json", "include": ["child/**/*"] }"#,
        );

        let cfg = load(&ts).unwrap();
        assert_eq!(
            cfg.include.as_deref(),
            Some(&["child/**/*".to_string()][..])
        );
    }

    #[test]
    fn child_without_include_inherits_parent_include() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "include": ["base/**/*"] }"#);
        write(&ts, r#"{ "extends": "./base.json" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.include.as_deref(), Some(&["base/**/*".to_string()][..]));
    }

    #[test]
    fn package_extends_via_node_modules() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/@tsconfig/svelte");
        fs::create_dir_all(&pkg_dir).unwrap();
        let pkg_ts = pkg_dir.join("tsconfig.json");
        write(
            &pkg_ts,
            r#"{ "compilerOptions": { "strict": true, "target": "ES2020" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "@tsconfig/svelte/tsconfig.json" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2020"));
    }

    #[test]
    fn package_extends_bare_name_defaults_to_tsconfig_json() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/my-tsconfig");
        fs::create_dir_all(&pkg_dir).unwrap();
        let pkg_ts = pkg_dir.join("tsconfig.json");
        write(&pkg_ts, r#"{ "compilerOptions": { "target": "ES2022" } }"#);

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "my-tsconfig" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
    }

    #[test]
    fn package_extends_walks_up_to_find_node_modules() {
        let tmp = tempdir().unwrap();
        let outer_nm = tmp.path().join("node_modules/my-tsconfig");
        fs::create_dir_all(&outer_nm).unwrap();
        write(
            &outer_nm.join("tsconfig.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        // Nested project has no node_modules of its own.
        let project = tmp.path().join("apps/inner/project");
        fs::create_dir_all(&project).unwrap();
        let ts = project.join("tsconfig.json");
        write(&ts, r#"{ "extends": "my-tsconfig" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    #[test]
    fn split_package_bare() {
        assert_eq!(split_package_and_subpath("my-pkg"), ("my-pkg", None));
    }

    #[test]
    fn split_package_with_subpath() {
        assert_eq!(
            split_package_and_subpath("my-pkg/tsconfig.json"),
            ("my-pkg", Some("tsconfig.json"))
        );
    }

    #[test]
    fn split_scoped_bare() {
        assert_eq!(
            split_package_and_subpath("@scope/pkg"),
            ("@scope/pkg", None)
        );
    }

    #[test]
    fn split_scoped_with_subpath() {
        assert_eq!(
            split_package_and_subpath("@scope/pkg/tsconfig.json"),
            ("@scope/pkg", Some("tsconfig.json"))
        );
    }

    #[test]
    fn split_scoped_with_deep_subpath() {
        assert_eq!(
            split_package_and_subpath("@scope/pkg/a/b.json"),
            ("@scope/pkg", Some("a/b.json"))
        );
    }

    #[test]
    fn entry_path_preserved_through_merge() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, "{}");
        write(&ts, r#"{ "extends": "./base.json" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.path, ts.canonicalize().unwrap());
    }

    #[test]
    fn load_chain_returns_every_visited_file_bfs() {
        let tmp = tempdir().unwrap();
        let gp = tmp.path().join("grandparent.json");
        let p = tmp.path().join("parent.json");
        let c = tmp.path().join("tsconfig.json");
        write(&gp, r#"{ "compilerOptions": { "strict": true } }"#);
        write(
            &p,
            r#"{ "extends": "./grandparent.json", "compilerOptions": { "target": "ES2020" } }"#,
        );
        write(
            &c,
            r#"{ "extends": "./parent.json", "compilerOptions": { "target": "ES2022" } }"#,
        );

        let chain = load_chain(&c).unwrap();
        // BFS from entry: child, parent, grandparent.
        assert_eq!(chain.len(), 3);
        assert!(chain[0].path.ends_with("tsconfig.json"));
        assert!(chain[1].path.ends_with("parent.json"));
        assert!(chain[2].path.ends_with("grandparent.json"));
    }

    #[test]
    fn load_chain_follows_array_extends_in_order() {
        let tmp = tempdir().unwrap();
        let a = tmp.path().join("a.json");
        let b = tmp.path().join("b.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&a, r#"{ "compilerOptions": { "strict": true } }"#);
        write(&b, r#"{ "compilerOptions": { "target": "ES2022" } }"#);
        write(&ts, r#"{ "extends": ["./a.json", "./b.json"] }"#);

        let chain = load_chain(&ts).unwrap();
        assert_eq!(chain.len(), 3);
        assert!(chain[0].path.ends_with("tsconfig.json"));
        assert!(chain[1].path.ends_with("a.json"));
        assert!(chain[2].path.ends_with("b.json"));
    }

    #[test]
    fn load_chain_substitutes_config_dir_per_file() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join("configs");
        let child_dir = tmp.path().join("project");
        fs::create_dir_all(&base_dir).unwrap();
        fs::create_dir_all(&child_dir).unwrap();
        let base = base_dir.join("base.json");
        let ts = child_dir.join("tsconfig.json");
        write(
            &base,
            r#"{ "compilerOptions": { "rootDirs": ["${configDir}/src"] } }"#,
        );
        write(&ts, r#"{ "extends": "../configs/base.json" }"#);

        let chain = load_chain(&ts).unwrap();
        // Parent's rootDirs should have ${configDir} resolved against
        // base's dir, not child's.
        let expected = base_dir.canonicalize().unwrap().join("src");
        let base_entry = chain
            .iter()
            .find(|f| f.path.ends_with("base.json"))
            .unwrap();
        assert_eq!(
            base_entry.compiler_options.root_dirs,
            vec![expected.to_str().unwrap()]
        );
    }

    #[test]
    fn load_chain_skips_unreadable_extends_without_failing() {
        let tmp = tempdir().unwrap();
        let ts = tmp.path().join("tsconfig.json");
        // Extends a file that doesn't exist. load() errors; load_chain
        // is best-effort and returns just the entry.
        write(&ts, r#"{ "extends": "./missing.json" }"#);

        let chain = load_chain(&ts).unwrap();
        assert_eq!(chain.len(), 1);
        assert!(chain[0].path.ends_with("tsconfig.json"));
    }

    #[test]
    fn deep_chain_merges_correctly() {
        let tmp = tempdir().unwrap();
        // grandparent → parent → child
        let gp = tmp.path().join("grandparent.json");
        let p = tmp.path().join("parent.json");
        let c = tmp.path().join("tsconfig.json");
        write(
            &gp,
            r#"{ "compilerOptions": { "strict": true, "target": "ES5" } }"#,
        );
        write(
            &p,
            r#"{ "extends": "./grandparent.json", "compilerOptions": { "target": "ES2018" } }"#,
        );
        write(
            &c,
            r#"{ "extends": "./parent.json", "compilerOptions": { "target": "ES2022" } }"#,
        );

        let cfg = load(&c).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true)); // from grandparent
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022")); // from child
    }
}
