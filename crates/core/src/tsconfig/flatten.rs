//! Flatten a TS project-references solution into its per-reference
//! contributions.
//!
//! Motivating use-case (from `notes/NEXT.md`'s solution-style redirect
//! sibling-visibility gap): the CLI redirects a solution-shaped root
//! (`files: []` + `references: [...]` + no `include`) to a sub-project
//! with real `compilerOptions.paths`. The overlay built around that
//! sub-project's chain has `include` scoped to the sub-project's tree,
//! so transitive imports into sibling referenced projects fire tsgo's
//! "File not listed within project" error.
//!
//! A full `tsc -b`-style build isn't feasible in our overlay — it
//! requires pre-built `.d.ts` outputs from each composite project.
//! Instead, we project each referenced project's OWN tsconfig into a
//! flattened shape and the overlay unions sibling-project
//! `include`/`exclude`/`paths` on top of the sub-project's own, so
//! sibling source files match an `include` glob and tsgo admits them.
//!
//! The overlay ONLY consumes references that point at a directory (or
//! its default `tsconfig.json`). References pointing at a specific
//! config file (`tsconfig.playwright.json`) are included — per-file
//! references are used by the user to narrow a project's scope, and
//! their `include` shapes we respect directly.

use std::path::{Path, PathBuf};

use super::load::{LoadError, load_chain};
use super::{TsConfigFile, parse_file};

/// One entry per reference in a solution-style tsconfig, with the
/// relevant fields projected from that reference's own `extends`
/// chain.
///
/// Path-valued fields are resolved to absolute paths against the
/// declaring config's dir (or `baseUrl` where appropriate). Pattern-
/// valued fields (`include` / `exclude`) are preserved as the user
/// wrote them — the overlay builder anchors them against `project_dir`
/// because absolute-glob resolution here would lose the user's intent
/// (a relative `./src/**` is rooted at the referenced project, not
/// the solution root).
#[derive(Debug, Clone)]
pub struct FlattenedReference {
    /// Absolute path to the referenced tsconfig file (not the dir).
    pub config_path: PathBuf,
    /// Absolute path to the project's directory (i.e. `config_path`'s
    /// parent). Overlay uses this as the anchor for relative
    /// `include` / `exclude` patterns.
    pub project_dir: PathBuf,
    /// Effective `include` patterns from the first config in the
    /// reference's chain that declares them, as the user wrote them.
    /// Empty vec when no config in the chain declared `include` — the
    /// overlay should fall back to a sensible default like
    /// `**/*.ts` + `**/*.d.ts` rooted at `project_dir`.
    pub include: Vec<String>,
    /// Effective `exclude` patterns. Same resolution rules as include.
    pub exclude: Vec<String>,
    /// Path aliases. Each value is an absolute path — resolved against
    /// the declaring config's `baseUrl` (or its dir when baseUrl is
    /// absent). BFS per-pattern first-wins across the reference's
    /// extends chain.
    pub paths: std::collections::BTreeMap<String, Vec<PathBuf>>,
    /// Effective `compilerOptions.types` from the first config in the
    /// reference's chain that declares them. Empty when no config in
    /// the chain sets `types`. Overlay unions these with the user
    /// workspace's own `types` so sibling projects that depend on
    /// `@types/<pkg>` (e.g. the `chrome` extension namespace) see
    /// their ambient declarations when tsgo checks files pulled in
    /// from them.
    pub types: Vec<String>,
    /// Effective `compilerOptions.lib` — same first-non-empty rule as
    /// `types`. Each sibling may declare a different lib set (a web
    /// project using `["DOM"]` next to an extension project using
    /// `["WebWorker"]`), and the overlay unions them so symbols from
    /// either lib resolve when sibling files are checked.
    pub lib: Vec<String>,
}

/// Parse a solution-style tsconfig, walk its `references[]`, and
/// return a [`FlattenedReference`] for each referenced project.
///
/// Returns `Ok(empty)` when `solution_root` is NOT solution-style
/// (i.e. has its own `include` or `files` or no references). That
/// lets callers invoke unconditionally — the non-solution case is a
/// zero-cost no-op.
///
/// References whose target doesn't exist on disk, whose tsconfig
/// can't be parsed, or whose chain fails to load are skipped
/// silently. Errors on the solution root itself surface as
/// [`LoadError::Parse`].
pub fn flatten_references(solution_root: &Path) -> Result<Vec<FlattenedReference>, LoadError> {
    let solution = parse_file(solution_root)?;
    if !solution.is_solution_style() {
        return Ok(Vec::new());
    }
    let solution_dir = solution.config_dir().to_path_buf();
    let mut out: Vec<FlattenedReference> = Vec::new();
    for reference in &solution.references {
        if let Some(r) = resolve_reference(&reference.path, &solution_dir) {
            out.push(r);
        }
    }
    Ok(out)
}

/// Flatten every reference in `config`'s own chain, TRANSITIVELY —
/// each referenced project's own `references[]` is walked too.
///
/// Used by the overlay when the CLI has redirected into a
/// sub-project. The sub-project's tsconfig declares direct refs; each
/// of those may itself reference further siblings. Without the
/// transitive walk, overlay include coverage misses files that
/// a direct ref imports from an indirect ref (common in monorepos
/// where `packages/types` references `packages/db`, and the
/// sub-project imports from types).
///
/// Cycles short-circuit via a visited-set keyed on canonical config
/// path. Returns an empty vec when nothing in the chain declared
/// references or when the entry config fails to load.
pub fn flatten_references_from_chain(entry: &Path) -> Vec<FlattenedReference> {
    let chain = match load_chain(entry) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut out: Vec<FlattenedReference> = Vec::new();
    let mut seen: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut queue: std::collections::VecDeque<(String, PathBuf)> =
        std::collections::VecDeque::new();
    // Seed with direct refs from every config in the entry's chain.
    for file in &chain {
        let dir = file.config_dir().to_path_buf();
        for reference in &file.references {
            queue.push_back((reference.path.clone(), dir.clone()));
        }
    }
    // Cap depth to keep pathological ref loops from running away.
    // Real monorepos rarely exceed 3-4 levels of transitive ref depth.
    let mut hops = 0usize;
    while let Some((ref_path, declaring_dir)) = queue.pop_front() {
        hops += 1;
        if hops > 256 {
            break;
        }
        let Some(r) = resolve_reference(&ref_path, &declaring_dir) else {
            continue;
        };
        if !seen.insert(r.config_path.clone()) {
            continue;
        }
        // Enqueue the flattened ref's OWN transitive references.
        // Re-loading the chain here is cheap — parse_file is fast
        // and we want the full extends chain's references[] mixed in
        // (a ref's tsconfig may extend a base that declares more
        // references).
        if let Ok(ref_chain) = load_chain(&r.config_path) {
            for rf in &ref_chain {
                let dir = rf.config_dir().to_path_buf();
                for reference in &rf.references {
                    queue.push_back((reference.path.clone(), dir.clone()));
                }
            }
        }
        out.push(r);
    }
    out
}

/// Shared resolution: take a reference's raw `path` string and the
/// declaring config's directory; produce a [`FlattenedReference`]
/// for it, or `None` on any error (missing file, malformed config,
/// etc.).
fn resolve_reference(raw_path: &str, declaring_dir: &Path) -> Option<FlattenedReference> {
    let ref_path = if Path::new(raw_path).is_absolute() {
        PathBuf::from(raw_path)
    } else {
        declaring_dir.join(raw_path)
    };
    let (config_path, project_dir) = if ref_path.is_dir() {
        (ref_path.join("tsconfig.json"), ref_path.clone())
    } else if ref_path.is_file() {
        let parent = ref_path.parent()?.to_path_buf();
        (ref_path.clone(), parent)
    } else {
        return None;
    };
    if !config_path.is_file() {
        return None;
    }
    let chain = load_chain(&config_path).ok()?;
    let include = first_non_empty(&chain, |f| f.include.as_deref()).unwrap_or_default();
    let exclude = first_non_empty(&chain, |f| f.exclude.as_deref()).unwrap_or_default();
    let paths = resolve_paths_bfs(&chain);
    let types =
        first_non_empty(&chain, |f| f.compiler_options.types.as_deref()).unwrap_or_default();
    let lib = first_non_empty_raw_strings(&chain, "lib");
    Some(FlattenedReference {
        config_path,
        project_dir,
        include,
        exclude,
        paths,
        types,
        lib,
    })
}

/// Pull a string-array compilerOption out of the typed struct's `raw`
/// passthrough — for fields we don't explicitly parse. `lib` is the
/// common one; tsgo's list of accepted values is large and versioned,
/// so we just echo the user's exact entries.
fn first_non_empty_raw_strings(chain: &[TsConfigFile], key: &str) -> Vec<String> {
    for file in chain {
        if let Some(serde_json::Value::Array(a)) = file.compiler_options.raw.get(key) {
            let values: Vec<String> = a
                .iter()
                .filter_map(|v| v.as_str().map(str::to_owned))
                .collect();
            if !values.is_empty() {
                return values;
            }
        }
    }
    Vec::new()
}

/// Return the first-declaring config's values for a multi-string
/// field (include, exclude). Matches TS's semantics where the field
/// is replaced wholesale by the inner config when set.
fn first_non_empty<F>(chain: &[TsConfigFile], get: F) -> Option<Vec<String>>
where
    F: Fn(&TsConfigFile) -> Option<&[String]>,
{
    for file in chain {
        if let Some(values) = get(file)
            && !values.is_empty()
        {
            return Some(values.to_vec());
        }
    }
    None
}

/// BFS per-pattern first-wins across the chain, resolving each value
/// to an absolute path against the declaring config's baseUrl (or
/// dir when baseUrl is absent).
fn resolve_paths_bfs(chain: &[TsConfigFile]) -> std::collections::BTreeMap<String, Vec<PathBuf>> {
    use std::collections::BTreeMap;
    let mut out: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for file in chain {
        let dir = file.config_dir();
        let base_url = match file.compiler_options.base_url.as_deref() {
            Some(b) if Path::new(b).is_absolute() => PathBuf::from(b),
            Some(b) => dir.join(b),
            None => dir.to_path_buf(),
        };
        for (pattern, values) in &file.compiler_options.paths {
            if out.contains_key(pattern) {
                continue; // inner wins
            }
            let resolved: Vec<PathBuf> = values
                .iter()
                .map(|v| {
                    if Path::new(v).is_absolute() {
                        PathBuf::from(v)
                    } else {
                        base_url.join(v)
                    }
                })
                .map(|p| normalize(&p))
                .collect();
            if !resolved.is_empty() {
                out.insert(pattern.clone(), resolved);
            }
        }
    }
    out
}

/// Collapse `..` segments without filesystem access. Duplicated from
/// `svn-typecheck::overlay` to avoid a dependency cycle; the logic is
/// trivial (no follow-symlink, no canonicalize).
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
    fn non_solution_root_returns_empty() {
        // A config with its own `include` is NOT solution-style.
        // flatten_references should bail quietly.
        let tmp = tempdir().unwrap();
        let ts = tmp.path().join("tsconfig.json");
        write(
            &ts,
            r#"{
                "compilerOptions": { "strict": true },
                "include": ["src/**/*.ts"]
            }"#,
        );
        let out = flatten_references(&ts).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn dir_reference_uses_default_tsconfig_and_pulls_its_include() {
        // Solution → { path: "./sub" } → sub/tsconfig.json with its
        // own include/exclude/paths. Flattened form carries those
        // through.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let sub_ts = root.join("sub/tsconfig.json");
        write(
            &sub_ts,
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": { "@app/*": ["./src/*"] }
                },
                "include": ["src/**/*.ts", "types/**/*.d.ts"],
                "exclude": ["src/fixtures/**/*"]
            }"#,
        );

        let root_ts = root.join("tsconfig.json");
        write(
            &root_ts,
            r#"{
                "files": [],
                "references": [{ "path": "./sub" }]
            }"#,
        );

        let refs = flatten_references(&root_ts).unwrap();
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.project_dir, root.join("sub"));
        assert_eq!(
            r.config_path,
            root.join("sub/tsconfig.json").canonicalize().unwrap()
        );
        assert_eq!(r.include, vec!["src/**/*.ts", "types/**/*.d.ts"]);
        assert_eq!(r.exclude, vec!["src/fixtures/**/*"]);
        let app_paths = r.paths.get("@app/*").unwrap();
        assert_eq!(app_paths, &[root.join("sub/src/*")]);
    }

    #[test]
    fn file_reference_points_at_specific_tsconfig_variant() {
        // Solution reference to a specific file
        // (tsconfig.playwright.json), NOT a directory. project_dir is
        // the file's parent; config_path IS the specified file; its
        // own include is preserved — NOT the default tsconfig.json's.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write(
            &root.join("app/tsconfig.json"),
            r#"{
                "compilerOptions": {},
                "include": ["src/**/*.ts"]
            }"#,
        );
        write(
            &root.join("app/tsconfig.playwright.json"),
            r#"{
                "extends": "./tsconfig.json",
                "include": ["playwright/**/*.ts"],
                "exclude": ["playwright/fixtures/**/*"]
            }"#,
        );
        write(
            &root.join("tsconfig.json"),
            r#"{
                "files": [],
                "references": [{ "path": "./app/tsconfig.playwright.json" }]
            }"#,
        );

        let refs = flatten_references(&root.join("tsconfig.json")).unwrap();
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.project_dir, root.join("app"));
        // The file ref's OWN include wins (TS semantics: inner wins
        // for include — and the playwright config declares one).
        assert_eq!(r.include, vec!["playwright/**/*.ts"]);
        assert_eq!(r.exclude, vec!["playwright/fixtures/**/*"]);
    }

    #[test]
    fn missing_reference_skipped_silently() {
        // Reference target doesn't exist on disk. Should not error.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        write(
            &root.join("tsconfig.json"),
            r#"{
                "files": [],
                "references": [
                    { "path": "./missing" },
                    { "path": "./present" }
                ]
            }"#,
        );
        write(
            &root.join("present/tsconfig.json"),
            r#"{
                "include": ["src/**/*"]
            }"#,
        );

        let refs = flatten_references(&root.join("tsconfig.json")).unwrap();
        assert_eq!(refs.len(), 1);
        assert_eq!(refs[0].project_dir, root.join("present"));
    }

    #[test]
    fn paths_inherit_through_reference_chain() {
        // The referenced project extends a base that declares paths.
        // Flattened form picks up inherited paths via BFS first-wins.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write(
            &root.join("tsconfig.base.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "inherited/*": ["./base-target/*"]
                    }
                }
            }"#,
        );
        write(
            &root.join("sub/tsconfig.json"),
            r#"{
                "extends": "../tsconfig.base.json",
                "compilerOptions": {
                    "paths": {
                        "own/*": ["./src/*"]
                    }
                },
                "include": ["src/**/*"]
            }"#,
        );
        write(
            &root.join("tsconfig.json"),
            r#"{
                "files": [],
                "references": [{ "path": "./sub" }]
            }"#,
        );

        let refs = flatten_references(&root.join("tsconfig.json")).unwrap();
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        // Own paths from sub itself.
        assert!(r.paths.contains_key("own/*"));
        // Inherited paths from base, resolved against base's dir (not
        // sub's).
        assert!(r.paths.contains_key("inherited/*"));
        assert_eq!(
            r.paths["inherited/*"],
            vec![root.join("base-target/*")],
            "inherited path should resolve against base's dir, not sub's",
        );
    }

    #[test]
    fn types_and_lib_flow_through_reference_chain() {
        // Sibling extension project declaring its own types + lib.
        // Real-world pattern: a web app references an extension
        // sub-project (which wants @types/chrome); the overlay needs
        // to carry those through.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write(
            &root.join("extension/tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "types": ["chrome", "node"],
                    "lib": ["ES2024", "DOM"]
                },
                "include": ["**/*.ts"]
            }"#,
        );
        write(
            &root.join("tsconfig.json"),
            r#"{
                "files": [],
                "references": [{ "path": "./extension" }]
            }"#,
        );

        let refs = flatten_references(&root.join("tsconfig.json")).unwrap();
        assert_eq!(refs.len(), 1);
        let r = &refs[0];
        assert_eq!(r.types, vec!["chrome".to_string(), "node".to_string()]);
        assert_eq!(r.lib, vec!["ES2024".to_string(), "DOM".to_string()]);
    }
}
