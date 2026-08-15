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
//! `include`/`exclude` on top of the sub-project's own, so sibling
//! source files match an `include` glob and tsgo admits them. A
//! referenced project's `paths` are deliberately NOT projected — they
//! have no effect on the project referencing it (the compiler applies
//! only the compiling project's own map).
//!
//! The overlay ONLY consumes references that point at a directory (or
//! its default `tsconfig.json`). References pointing at a specific
//! config file (`tsconfig.playwright.json`) are included — per-file
//! references are used by the user to narrow a project's scope, and
//! their `include` shapes we respect directly.

use std::path::{Path, PathBuf};

use super::load::{LoadError, load_chain, winning_field, winning_patterns};
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
    /// Effective `include` patterns from the innermost config in the
    /// reference's chain that declares the field, as the user wrote
    /// them. A declared-but-empty `include: []` wins over a base's
    /// non-empty one (TS replaces the field wholesale on redeclare).
    /// Empty vec when either no config declared `include` or the
    /// innermost declaration was empty — in both cases the overlay
    /// falls back to a sensible default like `**/*.ts` + `**/*.d.ts`
    /// rooted at `project_dir`.
    pub include: Vec<String>,
    /// Effective `exclude` patterns. Same resolution rules as include.
    pub exclude: Vec<String>,
    /// Effective `compilerOptions.types` from the innermost config in
    /// the reference's chain that declares the field. Empty when no
    /// config in the chain sets `types` (or the innermost declaration
    /// was empty). Overlay unions these with the user workspace's own
    /// `types` so sibling projects that depend on `@types/<pkg>`
    /// (e.g. the `chrome` extension namespace) see their ambient
    /// declarations when tsgo checks files pulled in from them.
    pub types: Vec<String>,
    /// Effective `compilerOptions.typeRoots` from the reference's own
    /// chain, resolved to absolute paths against the declaring config's
    /// directory. Empty when no config in the chain sets `typeRoots`.
    /// The overlay probes this project's `types` entries in THESE
    /// roots — a sibling keeping ambients in a custom directory must
    /// not have them filtered against the entry chain's roots.
    pub type_roots: Vec<PathBuf>,
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
    // Enqueue-time dedupe keyed on the normalized ref target: skips a
    // re-stat + re-parse for true duplicate targets of identical
    // spelling (diamond-shaped ref graphs). `seen` (keyed on the
    // resolved config_path) still backstops the dir/file-alias case,
    // so the returned set and its first-occurrence order are unchanged.
    let mut enqueued: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let push = |queue: &mut std::collections::VecDeque<(String, PathBuf)>,
                enqueued: &mut std::collections::HashSet<PathBuf>,
                raw: &str,
                dir: &Path| {
        let p = if Path::new(raw).is_absolute() {
            PathBuf::from(raw)
        } else {
            dir.join(raw)
        };
        if enqueued.insert(normalize(&p)) {
            queue.push_back((raw.to_string(), dir.to_path_buf()));
        }
    };
    // Seed with the ENTRY config's own references only.
    //
    // `references` is not inherited through `extends` — TypeScript reads
    // it from the config being loaded and nothing else, which
    // `load.rs`'s merge already implements (`base.references =
    // child.references`). Seeding from every config in the chain
    // contradicted that: a monorepo whose tsconfig.base.json carries a
    // references[] array pulled whole sibling projects' files, paths and
    // types into the overlay, and reported their errors, for a graph the
    // compiler does not traverse at all.
    if let Some(entry) = chain.first() {
        let dir = entry.config_dir().to_path_buf();
        for reference in &entry.references {
            push(&mut queue, &mut enqueued, &reference.path, &dir);
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
        // Enqueue the flattened ref's OWN transitive references — again
        // only the ones it declares itself, for the same reason.
        if let Ok(ref_chain) = load_chain(&r.config_path)
            && let Some(entry) = ref_chain.first()
        {
            let dir = entry.config_dir().to_path_buf();
            for reference in &entry.references {
                push(&mut queue, &mut enqueued, &reference.path, &dir);
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
    // Collapse `..`/`.` segments so the stored spelling is canonical:
    // `config_path` backs the visited-set key in the caller and
    // `project_dir` anchors include globs, so both must be free of
    // redundant segments to dedupe and resolve consistently.
    let config_path = normalize(&config_path);
    let project_dir = normalize(&project_dir);
    if !config_path.is_file() {
        return None;
    }
    let chain = load_chain(&config_path).ok()?;
    // `winning_patterns` applies TS's replace-on-redeclare precedence:
    // the innermost config that DECLARES the field wins, including a
    // declared-but-empty `[]` — which replaces a base's non-empty
    // value rather than falling through to it.
    let declared = |get: fn(&TsConfigFile) -> Option<&[String]>| {
        winning_patterns(&chain, get)
            .map(|(_, values)| values.to_vec())
            .unwrap_or_default()
    };
    let include = declared(|f| f.include.as_deref());
    let exclude = declared(|f| f.exclude.as_deref());
    let types = declared(|f| f.compiler_options.types.as_deref());
    // The sibling's own `typeRoots`, anchored on the config that
    // declares them — the sibling's `types` entries must be probed in
    // ITS roots, not the entry chain's, or a sibling that keeps its
    // ambients in a custom directory gets them all filtered out.
    let type_roots = winning_field(&chain, |f| f.compiler_options.type_roots.as_deref())
        .map(|(f, roots)| {
            let dir = f.config_dir();
            roots
                .iter()
                .map(|r| {
                    if Path::new(r).is_absolute() {
                        normalize(Path::new(r))
                    } else {
                        normalize(&dir.join(r))
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    Some(FlattenedReference {
        config_path,
        project_dir,
        include,
        exclude,
        types,
        type_roots,
    })
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
        // own include/exclude. Flattened form carries those through.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        let sub_ts = root.join("sub/tsconfig.json");
        write(
            &sub_ts,
            r#"{
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
    fn declared_empty_include_wins_over_base() {
        // TS `extends` semantics are replace-on-redeclare: a child
        // that writes `"include": []` has DECLARED the field, and its
        // empty value replaces the base's non-empty one. Falling
        // through to the base here would admit files the compiler
        // does not include.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write(
            &root.join("tsconfig.base.json"),
            r#"{
                "include": ["src/**/*.ts"]
            }"#,
        );
        write(
            &root.join("sub/tsconfig.json"),
            r#"{
                "extends": "../tsconfig.base.json",
                "include": [],
                "files": ["main.ts"]
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
        assert!(
            refs[0].include.is_empty(),
            "child's explicit `include: []` must replace the base's, got {:?}",
            refs[0].include,
        );
    }

    #[test]
    fn types_flow_through_reference_chain() {
        // Sibling extension project declaring its own types.
        // Real-world pattern: a web app references an extension
        // sub-project (which wants @types/chrome); the overlay needs
        // to carry those through.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write(
            &root.join("extension/tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "types": ["chrome", "node"]
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
        // No typeRoots declared anywhere in the sibling's chain.
        assert!(r.type_roots.is_empty());
    }

    #[test]
    fn type_roots_anchor_on_the_declaring_sibling_config() {
        // A sibling keeping ambients in its own typings/ dir: its
        // `types` entries must be probed in ITS roots, so the roots are
        // projected absolute against the sibling config's directory.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write(
            &root.join("extension/tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "typeRoots": ["./typings"],
                    "types": ["globals"]
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
        assert_eq!(r.type_roots, vec![root.join("extension/typings")]);
    }
}
