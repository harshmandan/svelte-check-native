//! Generate the overlay tsconfig that tsgo runs against.
//!
//! The overlay extends the user's tsconfig and re-points it at the
//! generated `.svelte.ts` files in the cache. Forces the flags tsgo needs
//! to consume our virtual files (`allowArbitraryExtensions`, `noEmit`,
//! incremental build info location).
//!
//! ### rootDirs merging
//!
//! TS treats `rootDirs` as an array — and arrays do NOT merge across the
//! `extends` chain (inner config wins outright). That means just setting
//! a child `rootDirs` here would clobber whatever the user's tsconfig had
//! (commonly SvelteKit's `[".." , "./types"]`).
//!
//! To keep relative imports resolving from generated `++Foo.svelte.ts`
//! files in the overlay back to the original source tree, we compute the
//! union of:
//!   - `<overlay>/svelte`        — where our generated `.ts` files live
//!   - every `rootDirs` entry from the user's `extends` chain (resolved
//!     to absolute paths so the absolute-vs-relative distinction is gone)
//!   - the workspace root         — fallback for projects that don't
//!     declare `rootDirs` themselves
//!
//! TS then virtually merges all those folders, so a relative import
//! `./loading-labels` from
//! `<overlay>/svelte/src/lib/components/ai/++AssistantOverlay.svelte.ts`
//! ALSO resolves against
//! `<workspace>/src/lib/components/ai/loading-labels`.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use svn_core::tsconfig::{
    FlattenedReference, TsConfigFile, flatten_references_from_chain, load_chain,
};

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
    kit_overlay_sources: &[std::path::PathBuf],
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

    // Walk the user's extends chain once via the canonical loader.
    // Every derived field the overlay needs (paths, rootDirs, include,
    // exclude, types) is computed by iterating this single Vec — no
    // parallel JSON reads, no local extends resolver. The loader does
    // `${configDir}` substitution, `.json` inference, and
    // `node_modules/@tsconfig/...` walk-up for us.
    let chain: Vec<TsConfigFile> = load_chain(user_tsconfig).unwrap_or_default();

    // When the CLI redirected from a solution-style root, pull sibling
    // projects REFERENCED BY THE REDIRECT TARGET into the overlay so
    // transitive imports across projects reach tsgo as part of the
    // same program. Flattening the solution root's full references[]
    // would over-include — for a app-style repo where
    // `tsconfig.json` coordinates console + functions + packages,
    // type-checking console doesn't require functions code, yet
    // including functions' tsconfig pulls its strict-mode errors
    // into our output.
    //
    // The narrower rule: only follow `references[]` declared BY the
    // redirect target (or its extends chain). That matches the
    // user's own tsconfig's declaration of "these are the projects
    // whose types I need." Skip a reference pointing at the current
    // workspace — its own tsconfig chain already covers it via
    // `chain`. Empty vec for flat-project runs.
    let sibling_refs: Vec<FlattenedReference> = flatten_references_from_chain(user_tsconfig)
        .into_iter()
        .filter(|r| r.project_dir != layout.workspace)
        .collect();
    // Acknowledge but don't consume the solution root — the CLI
    // still passes it down for future expansion (e.g.
    // paths-level aliases from the solution root).
    let _ = layout.solution_root_tsconfig.as_deref();

    let mut root_dirs: Vec<String> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let push_root =
        |dir: &Path, out: &mut Vec<String>, seen: &mut std::collections::HashSet<String>| {
            let s = dir.to_string_lossy().to_string();
            if seen.insert(s.clone()) {
                out.push(s);
            }
        };
    // Overlay svelte subdir first — that's where generated files live.
    push_root(layout.svelte_dir.as_path(), &mut root_dirs, &mut seen);
    // Workspace root second — so any relative import from a generated
    // overlay file (e.g. `import x from '../stores/util.ts'` written
    // inside a .svelte) resolves through TS's rootDirs virtual merge
    // back to the real source tree. Use the workspace explicitly: the
    // cache root's parent is no longer the workspace ever since we
    // moved the cache under node_modules/.cache/.
    push_root(layout.workspace.as_path(), &mut root_dirs, &mut seen);
    // UNION `rootDirs` across the whole extends chain. TS semantics
    // REPLACE the field when a child declares it, but we need the
    // widest possible virtual-merge in the overlay so every relative
    // import the user could have written still resolves back to the
    // real source tree.
    for file in &chain {
        let dir = file.config_dir();
        for rd in &file.compiler_options.root_dirs {
            let resolved = if Path::new(rd).is_absolute() {
                PathBuf::from(rd)
            } else {
                dir.join(rd)
            };
            push_root(normalize(&resolved).as_path(), &mut root_dirs, &mut seen);
        }
    }

    // Path aliases: walk the chain and apply per-pattern first-wins
    // (inner config beats outer for the same key — the walker is BFS
    // from the entry, so chain[0] is the innermost). Prepend a
    // cache-mirror candidate to each value-list so a path-mapped import
    // like `$lib/foo/Bar.svelte.ts` first tries our generated overlay
    // file and falls back to the source location if not found.
    // Without this, path-mapped Svelte imports skip rootDirs entirely
    // and never reach our overlay (rootDirs only kicks in for raw
    // relative paths).
    let mut paths_map: serde_json::Map<String, Value> = serde_json::Map::new();
    let mut paths_keys_order: Vec<String> = Vec::new();
    let mut paths_accumulated: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for file in &chain {
        let dir = file.config_dir();
        // baseUrl (if set) resolves relative `paths` values.
        let base_url = match file.compiler_options.base_url.as_deref() {
            Some(b) if Path::new(b).is_absolute() => PathBuf::from(b),
            Some(b) => dir.join(b),
            None => dir.to_path_buf(),
        };
        for (pattern, values) in &file.compiler_options.paths {
            if paths_accumulated.contains_key(pattern) {
                continue; // inner wins per pattern
            }
            let mut resolved: Vec<String> = Vec::new();
            for v in values {
                let abs = if Path::new(v).is_absolute() {
                    PathBuf::from(v)
                } else {
                    base_url.join(v)
                };
                resolved.push(normalize(&abs).to_string_lossy().into_owned());
            }
            if !resolved.is_empty() {
                paths_keys_order.push(pattern.clone());
                paths_accumulated.insert(pattern.clone(), resolved);
            }
        }
    }
    // Sibling-project paths: only fill in patterns NOT already
    // declared by the redirect target's chain. Inner-wins policy
    // preserves user intent when the redirect target has its own
    // alias for the same pattern; sibling projects only contribute
    // aliases that the redirect target hasn't claimed.
    for sibling in &sibling_refs {
        for (pattern, values) in &sibling.paths {
            if paths_accumulated.contains_key(pattern) {
                continue;
            }
            let resolved: Vec<String> = values
                .iter()
                .map(|v| v.to_string_lossy().into_owned())
                .collect();
            if !resolved.is_empty() {
                paths_keys_order.push(pattern.clone());
                paths_accumulated.insert(pattern.clone(), resolved);
            }
        }
    }
    for pattern in paths_keys_order {
        let values = paths_accumulated.remove(&pattern).unwrap_or_default();
        let mut merged: Vec<String> = Vec::new();
        for v in &values {
            if let Some(m) = mirror_into_overlay(layout, v) {
                if !merged.iter().any(|x| x == &m) {
                    merged.push(m);
                }
            }
        }
        for v in values {
            if !merged.iter().any(|x| x == &v) {
                merged.push(v);
            }
        }
        paths_map.insert(
            pattern,
            Value::Array(merged.into_iter().map(Value::String).collect()),
        );
    }

    let mut compiler_options = serde_json::Map::new();
    compiler_options.insert("noEmit".into(), json!(true));
    compiler_options.insert("allowArbitraryExtensions".into(), json!(true));
    // Disable case-consistency enforcement in the overlay. The user's
    // own tsconfig typically turns it on, and for pure .ts/.tsx code
    // that's the right default — but our overlay pipeline writes
    // generated files to a cache dir mirrored from user paths. On
    // case-insensitive filesystems (default macOS), resolving
    // `./Code.svelte` (user import) via bundler auto-extension can
    // case-insensitively hit a sibling `code.svelte.ts` runes module,
    // and tsgo then logs a TS1149 "file name differs only in casing"
    // against the user. Upstream svelte-check uses an in-memory
    // compiler host and sidesteps this entirely. We don't have that
    // luxury, so we relax the check project-wide in the overlay.
    // User's actual case-inconsistency bugs still get caught by
    // running tsc directly on their source; this only affects the
    // overlay pass.
    compiler_options.insert("forceConsistentCasingInFileNames".into(), json!(false));
    // `allowImportingTsExtensions` is INHERITED, not forced. Whatever
    // the user sets in their tsconfig carries through. Setting it to
    // `true` unconditionally here silently widened user-authored
    // `.ts`-extension imports that upstream svelte-check flags via
    // TS5097 — 44 such errors on bench/palacms alone. Upstream's own
    // overlay doesn't set the flag either; our `.svelte` overlay
    // resolution doesn't need it (handled by `allowArbitraryExtensions`
    // + the `.d.svelte.ts` ambient sidecars whose `.ts` re-exports are
    // legal under declaration-file rules regardless of the flag).
    compiler_options.insert("incremental".into(), json!(true));
    compiler_options.insert(
        "tsBuildInfoFile".into(),
        json!(layout.tsbuildinfo.to_string_lossy()),
    );
    compiler_options.insert("skipLibCheck".into(), json!(true));
    // tsgo has removed the legacy `node`/`node10` moduleResolution
    // values. Force `bundler` in the overlay so tsgo accepts the
    // project; the user's runtime moduleResolution setting is
    // unaffected (we never touch their files).
    compiler_options.insert("moduleResolution".into(), json!("bundler"));
    compiler_options.insert("module".into(), json!("esnext"));
    compiler_options.insert("rootDirs".into(), json!(root_dirs));
    // Filter the user's `types` to drop entries that don't resolve.
    // tsgo treats a missing `types` entry as a fatal TS2688 and stops
    // emitting diagnostics for the rest of the program — so a single
    // stale path (a build-time-generated .d.ts that hasn't been
    // regenerated yet, a typo, etc.) silently zeros our error count.
    // We override the inherited `types` with the surviving entries so
    // the user's intent is preserved without the fatal-abort.
    // `types`: inner wins (first config in the BFS chain that declares
    // the field). Filter against tsgo's resolution rules so a stale
    // entry can't fatally TS2688 and zero out our error count.
    if let Some(types) = chain
        .iter()
        .find(|f| f.compiler_options.types.is_some())
        .and_then(|f| {
            f.compiler_options.types.as_ref().map(|list| {
                let dir = f.config_dir();
                list.iter()
                    .filter(|t| is_resolvable_types_entry(t, dir))
                    .cloned()
                    .collect::<Vec<_>>()
            })
        })
    {
        compiler_options.insert("types".into(), json!(types));
    }
    if !paths_map.is_empty() {
        compiler_options.insert("paths".into(), Value::Object(paths_map));
        // Intentionally NOT setting `baseUrl`. TypeScript 5.0 removed
        // `baseUrl` as a top-level compiler option (TS5102) and tsgo
        // (the TS 7.0 dev preview that this binary targets) doesn't
        // require it for `paths` to resolve — every paths-target value
        // we emit is absolute. Setting baseUrl had a real, hidden
        // cost: tsgo silently suppresses diagnostic emission for
        // files outside `baseUrl`'s tree AND, in some configurations,
        // suppresses diagnostics on overlay files entirely. The TS5102
        // deprecation that tsgo emits when paths is set without
        // baseUrl is filtered as overlay noise in
        // svn-typecheck::map_diagnostic.
    }

    // Pull the user's `include` patterns into our overlay so tsgo also
    // type-checks standalone TS modules the user authored (route loaders,
    // hooks, $lib helpers, .svelte.ts rune-helper modules, etc.). Without
    // this our overlay only sees the generated .svelte.ts overlays plus
    // their transitive imports — anything the user `include`s but that's
    // not reached from a .svelte file goes unchecked.
    //
    // Patterns matching `*.svelte` are dropped: tsgo can't parse raw
    // .svelte files, and the .svelte content is already covered by the
    // generated overlays we list in `files`. Patterns are emitted as
    // absolute path globs so the tsconfig works regardless of the
    // overlay's location relative to the workspace.
    // Inner wins for include/exclude. `include` drops `.svelte`-only
    // patterns since our generated overlays in `files` already cover
    // them; `exclude` keeps everything (dropping a pattern opens a
    // hole).
    let mut user_includes = first_non_empty_patterns(&chain, |f| f.include.as_deref(), true);
    // Sibling-project includes: add each reference's own include
    // patterns, anchored at the reference's project_dir. Fall back to
    // `**/*.ts` + `**/*.d.ts` for references that don't declare their
    // own include (tsc's default when `include` is absent and `files`
    // is absent). `.svelte` patterns are dropped per the same rule as
    // the user's own includes. This is the sibling-visibility fix:
    // without it, a transitive import from the redirect target into a
    // sibling project fires tsgo's "File not listed within project".
    for sibling in &sibling_refs {
        let project_dir = sibling.project_dir.as_path();
        if sibling.include.is_empty() {
            for ext in ["ts", "d.ts"] {
                let glob = format!("{}/**/*.{}", project_dir.to_string_lossy(), ext);
                if !user_includes.contains(&glob) {
                    user_includes.push(glob);
                }
            }
        } else {
            for pat in &sibling.include {
                if is_svelte_only_pattern(pat) {
                    continue;
                }
                let resolved = if Path::new(pat).is_absolute() {
                    PathBuf::from(pat)
                } else {
                    project_dir.join(pat)
                };
                let glob = normalize(&resolved).to_string_lossy().into_owned();
                if !user_includes.contains(&glob) {
                    user_includes.push(glob);
                }
            }
        }
    }

    let mut overlay = serde_json::Map::new();
    overlay.insert("extends".into(), Value::String(extends_rel));
    overlay.insert("compilerOptions".into(), Value::Object(compiler_options));
    overlay.insert("files".into(), json!(files));
    if !user_includes.is_empty() {
        overlay.insert("include".into(), json!(user_includes));
    }
    // Exclude list. Two sources that must union:
    //
    // 1. The user's own `exclude` from their tsconfig chain — e.g.
    //    `playwright/fixtures/videos/**/*` for binary-named-`.ts`
    //    files. tsconfig semantics REPLACE (not merge) exclude when
    //    the child config declares one, so dropping this would let
    //    user-excluded content back into the program.
    // 2. Original Kit-file source paths that have an injected
    //    overlay at a mirrored cache path. Without this, tsgo
    //    loads BOTH the untyped original and the typed overlay.
    //
    // Only emit the field if at least one source contributed — an
    // empty `exclude` field in our overlay would clobber the user's
    // inherited exclude with an empty list.
    let mut excludes: Vec<String> =
        first_non_empty_patterns(&chain, |f| f.exclude.as_deref(), false);
    // Each sibling reference's own `exclude` patterns, anchored at
    // that reference's project_dir. Critical for preserving user
    // intent — sub-app's tsconfig.playwright.json excludes binary
    // `.ts` files under `./playwright/fixtures/videos/**/*`; without
    // propagating that, our widened include would pull them in and
    // fire tsgo "file appears to be binary" errors.
    for sibling in &sibling_refs {
        let project_dir = sibling.project_dir.as_path();
        for pat in &sibling.exclude {
            let resolved = if Path::new(pat).is_absolute() {
                PathBuf::from(pat)
            } else {
                project_dir.join(pat)
            };
            let glob = normalize(&resolved).to_string_lossy().into_owned();
            if !excludes.contains(&glob) {
                excludes.push(glob);
            }
        }
    }
    for p in kit_overlay_sources {
        excludes.push(p.to_string_lossy().into_owned());
    }
    if !excludes.is_empty() {
        overlay.insert("exclude".into(), json!(excludes));
    }
    Value::Object(overlay)
}

/// Map an absolute paths-target path INTO the overlay svelte tree. If
/// the input path is not under the workspace root, return None — the
/// mirror only makes sense for paths inside the project we generated
/// overlays for.
///
/// Resolves relative paths against the workspace root explicitly. The
/// cache root's parent stopped equalling the workspace once the cache
/// moved under `node_modules/.cache/`, so taking `layout.root.parent()`
/// would point at `node_modules/.cache/` and strip-prefix would fail
/// for every path-target the user actually declared.
fn mirror_into_overlay(layout: &CacheLayout, path_str: &str) -> Option<String> {
    let p = Path::new(path_str);
    let abs = if p.is_absolute() {
        p.to_path_buf()
    } else {
        layout.workspace.join(p)
    };
    let normalized = normalize(&abs);
    let rel = normalized.strip_prefix(&layout.workspace).ok()?;
    let mirrored = layout.svelte_dir.join(rel);
    Some(mirrored.to_string_lossy().into_owned())
}

/// First tsconfig in the BFS chain that declares `include` / `exclude`
/// wins — match TS's replace-on-child semantics. Each pattern is
/// resolved against the declaring config's dir so the overlay's
/// absolute-path globs work regardless of where the overlay tsconfig
/// itself lives.
///
/// `drop_svelte_only` filters patterns that only match raw `.svelte`
/// files — valid for `include` (our generated `.svelte.ts` overlays in
/// `files` already cover that surface) but NOT for `exclude`, where
/// dropping a pattern would open a hole.
fn first_non_empty_patterns<F>(
    chain: &[TsConfigFile],
    get: F,
    drop_svelte_only: bool,
) -> Vec<String>
where
    F: Fn(&TsConfigFile) -> Option<&[String]>,
{
    for file in chain {
        let Some(patterns) = get(file) else {
            continue;
        };
        let dir = file.config_dir();
        let mut out: Vec<String> = Vec::new();
        for s in patterns {
            if drop_svelte_only && is_svelte_only_pattern(s) {
                continue;
            }
            let resolved = if Path::new(s).is_absolute() {
                PathBuf::from(s)
            } else {
                dir.join(s)
            };
            out.push(normalize(&resolved).to_string_lossy().into_owned());
        }
        if !out.is_empty() {
            return out;
        }
    }
    Vec::new()
}

/// True when an include pattern is only meaningful for raw `.svelte`
/// files — those are handled by our overlay's generated `.svelte.ts`
/// files in the `files` array, so dropping the pattern keeps tsgo from
/// trying to load the original `.svelte` source as TypeScript.
fn is_svelte_only_pattern(pattern: &str) -> bool {
    let trimmed = pattern.trim_end_matches('/');
    trimmed.ends_with(".svelte")
}

/// True when a `types` entry will resolve under tsgo's lookup rules.
///
/// Entries fall into two buckets:
///
/// **Filesystem paths** — start with `.`, `..`, or `/`. Must point at an
/// existing file. tsgo's `types` lookup for relative entries does NOT
/// add `.d.ts` automatically when the path already includes an
/// extension, so we test the literal path first and `<path>.d.ts` as a
/// fallback. This is the narrow case where `types: ["./foo"]` is a
/// literal file reference.
///
/// **Package entries** — everything else. Includes:
///   - Bare names: `"node"`, `"svelte"`.
///   - Scoped names: `"@types/foo"`, `"@sveltejs/kit"`.
///   - Package-subpath entries: `"vite/client"`, `"vitest/globals"`,
///     `"@sveltejs/kit/types"`. The subpath component is resolved
///     internally by the package (via its `exports` map,
///     `typesVersions`, or bundled .d.ts layout) — we don't try to
///     second-guess which file it lands on. Checking that the package
///     itself is installed in the workspace's `node_modules` chain is
///     sufficient; tsgo does the rest.
///
/// This filtering exists because SvelteKit's auto-generated
/// `.svelte-kit/tsconfig.json` declares `types: ["node"]` even when the
/// host project doesn't actually depend on `@types/node` — without it,
/// tsgo treats the missing entry as fatal TS2688 and stops emitting
/// diagnostics for the entire program. The classifier has to keep
/// genuinely-installed entries (including subpaths like `vite/client`)
/// or user code loses its ambient types.
fn is_resolvable_types_entry(entry: &str, declaring_dir: &Path) -> bool {
    if is_filesystem_types_entry(entry) {
        let candidate = if Path::new(entry).is_absolute() {
            PathBuf::from(entry)
        } else {
            declaring_dir.join(entry)
        };
        if candidate.is_file() {
            return true;
        }
        let with_dts = candidate.with_extension(format!(
            "{}.d.ts",
            candidate.extension().and_then(|e| e.to_str()).unwrap_or(""),
        ));
        if with_dts.is_file() {
            return true;
        }
        let mut as_dts = candidate.clone();
        as_dts.as_mut_os_string().push(".d.ts");
        return as_dts.is_file();
    }
    let (pkg, _subpath) = split_package_entry(entry);
    package_types_entry_resolves(pkg, declaring_dir)
}

/// True when the entry should be treated as a filesystem path rather
/// than a package spec. Filesystem paths begin with `./`, `../`, or `/`
/// (POSIX-style absolute). Everything else — bare names, scoped names,
/// package subpaths — resolves through `node_modules`.
fn is_filesystem_types_entry(entry: &str) -> bool {
    entry.starts_with('.') || entry.starts_with('/')
}

/// Split a package-style `types` entry into its package root and the
/// (possibly empty) subpath.
///
/// Examples:
///   - `"node"` → `("node", "")`
///   - `"vite/client"` → `("vite", "client")`
///   - `"vitest/globals"` → `("vitest", "globals")`
///   - `"@sveltejs/kit"` → `("@sveltejs/kit", "")`
///   - `"@sveltejs/kit/types"` → `("@sveltejs/kit", "types")`
///
/// The package root is always the portion that lives directly under
/// `node_modules/`: for unscoped packages it's everything before the
/// first `/`, for scoped packages it's the first two segments.
fn split_package_entry(entry: &str) -> (&str, &str) {
    if let Some(rest) = entry.strip_prefix('@') {
        // Scoped package: @<scope>/<name>[/<subpath>]. Find the second
        // slash — that marks the boundary between package and subpath.
        let scope_end = match rest.find('/') {
            Some(idx) => idx,
            None => return (entry, ""),
        };
        let after_scope = &rest[scope_end + 1..];
        match after_scope.find('/') {
            Some(idx) => {
                let pkg_end = 1 + scope_end + 1 + idx;
                (&entry[..pkg_end], &entry[pkg_end + 1..])
            }
            None => (entry, ""),
        }
    } else {
        match entry.split_once('/') {
            Some((pkg, sub)) => (pkg, sub),
            None => (entry, ""),
        }
    }
}

/// True when a package-name `types` entry resolves to either an `@types`
/// package or a runtime package shipping its own .d.ts files in the
/// workspace's `node_modules` chain. Walks up from the declaring
/// tsconfig's directory; first match wins.
fn package_types_entry_resolves(name: &str, declaring_dir: &Path) -> bool {
    let mut cur: Option<&Path> = Some(declaring_dir);
    while let Some(dir) = cur {
        let nm = dir.join("node_modules");
        if nm.is_dir() {
            // Conventional types package: node_modules/@types/<name>.
            if nm.join("@types").join(name).join("package.json").is_file() {
                return true;
            }
            // Runtime package shipping its own types: node_modules/<name>.
            if nm.join(name).join("package.json").is_file() {
                return true;
            }
        }
        cur = dir.parent();
    }
    false
}

/// Collapse `..` segments without touching the filesystem. Pure path
/// arithmetic — necessary because `Path::canonicalize` requires the path
/// to exist, and rootDirs entries from extends chains often point at
/// locations that won't exist for every user.
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
        let overlay = build(&layout, &user_ts, &gen_files, &[]);

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
        let overlay = build(&layout, &user_ts, &[], &[]);
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
        let overlay = build(&layout, &user_ts, &gen_files, &[]);
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
        let overlay = build(&layout, &user_ts, &[], &[]);
        let files = overlay["files"].as_array().unwrap();
        assert_eq!(files.len(), 1);
        assert!(files[0].as_str().unwrap().ends_with("svelte-shims.d.ts"));
    }

    // ===== Canonical-loader-driven overlay behaviors ====================
    //
    // These write real tsconfigs into a tempdir and run `build()` end-to-
    // end through `load_chain`. Guards against regressions in the three
    // places the overlay's loader integration matters most:
    //
    //   * package `extends` via `node_modules/<pkg>/…`
    //   * `${configDir}` substitution per-declaring-file
    //   * array-form `extends` (TS 5.0+) merge order
    //
    // Each test sets up the minimal on-disk shape and asserts on the
    // overlay JSON that `build()` returns.

    use std::fs;
    use tempfile::tempdir;

    fn write_file(path: &Path, contents: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    #[test]
    fn build_overlay_inherits_paths_and_rootdirs_from_package_extends() {
        // Workspace tsconfig extends `@tsconfig/svelte` from a local
        // node_modules. The overlay builder should walk the package-
        // extends target, inherit its `paths` + `rootDirs`, and
        // project them into the overlay with absolute-path values.
        let tmp = tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();

        let pkg_ts = ws.join("node_modules/@tsconfig/svelte/tsconfig.json");
        write_file(
            &pkg_ts,
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "$lib": ["./src/lib"],
                        "$lib/*": ["./src/lib/*"]
                    },
                    "rootDirs": ["./extra-types"]
                }
            }"#,
        );

        let user_ts = ws.join("tsconfig.json");
        write_file(
            &user_ts,
            r#"{ "extends": "@tsconfig/svelte/tsconfig.json" }"#,
        );

        let layout = CacheLayout::for_workspace(&ws);
        let overlay = build(&layout, &user_ts, &[], &[]);

        let opts = &overlay["compilerOptions"];
        // rootDirs union includes svelte cache, workspace, AND the
        // inherited rootDirs entry, resolved against the package
        // tsconfig's dir (not the user's).
        let root_dirs: Vec<&str> = opts["rootDirs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_extra = ws
            .join("node_modules/@tsconfig/svelte/extra-types")
            .to_string_lossy()
            .into_owned();
        assert!(
            root_dirs.iter().any(|r| *r == expected_extra),
            "expected {expected_extra:?} in rootDirs, got {root_dirs:?}",
        );

        // paths inherit from the package extends and get projected with
        // a cache-mirror candidate prepended for each value.
        let paths = opts["paths"].as_object().unwrap();
        assert!(
            paths.contains_key("$lib"),
            "paths keys: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
        assert!(paths.contains_key("$lib/*"));
        let lib_values: Vec<&str> = paths["$lib"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_original = ws
            .join("node_modules/@tsconfig/svelte/src/lib")
            .to_string_lossy()
            .into_owned();
        assert!(
            lib_values.iter().any(|v| *v == expected_original),
            "original paths-target not present: {lib_values:?}",
        );
    }

    #[test]
    fn build_overlay_substitutes_configdir_to_declaring_files_dir() {
        // Base config uses `${configDir}` for both baseUrl and rootDirs;
        // the user extends it from a DIFFERENT directory. Overlay must
        // resolve the placeholder against the BASE's dir, not the user's.
        let tmp = tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();
        let base_dir = ws.join("configs");
        let project_dir = ws.join("project");

        let base_ts = base_dir.join("base.json");
        write_file(
            &base_ts,
            r#"{
                "compilerOptions": {
                    "baseUrl": "${configDir}/src",
                    "rootDirs": ["${configDir}/types"],
                    "paths": {
                        "$lib": ["./local/lib"],
                        "$abs": ["${configDir}/abs-target"]
                    }
                }
            }"#,
        );

        let user_ts = project_dir.join("tsconfig.json");
        write_file(&user_ts, r#"{ "extends": "../configs/base.json" }"#);

        let layout = CacheLayout::for_workspace(&project_dir);
        let overlay = build(&layout, &user_ts, &[], &[]);

        let opts = &overlay["compilerOptions"];

        // ${configDir} in rootDirs resolves to the base's dir.
        let root_dirs: Vec<&str> = opts["rootDirs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_types = base_dir.join("types").to_string_lossy().into_owned();
        assert!(
            root_dirs.iter().any(|r| *r == expected_types),
            "expected ${{configDir}}-resolved rootDirs entry {expected_types:?}, got {root_dirs:?}",
        );
        // Must NOT resolve against the user's dir.
        let wrong_types = project_dir.join("types").to_string_lossy().into_owned();
        assert!(
            !root_dirs.iter().any(|r| *r == wrong_types),
            "${{configDir}} wrongly resolved to user's dir: {wrong_types:?}",
        );

        // Paths: relative `./local/lib` resolves against base's baseUrl
        // (which itself is ${configDir}/src → base_dir/src). Absolute
        // `${configDir}/abs-target` resolves to base_dir/abs-target.
        let paths = opts["paths"].as_object().unwrap();
        let abs_values: Vec<&str> = paths["$abs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_abs = base_dir.join("abs-target").to_string_lossy().into_owned();
        assert!(
            abs_values.iter().any(|v| *v == expected_abs),
            "${{configDir}}-in-paths not resolved correctly: {abs_values:?}",
        );
    }

    #[test]
    fn build_overlay_preserves_array_extends_merge_order() {
        // `extends: ["./a.json", "./b.json"]` — TS 5.0+ semantics walk
        // left-to-right; the entry itself wins over array members, and
        // later array entries override earlier ones. The overlay's
        // `paths` first-wins should reflect BFS order: entry > b > a.
        let tmp = tempdir().unwrap();
        let ws = tmp.path().canonicalize().unwrap();

        write_file(
            &ws.join("a.json"),
            r#"{
                "compilerOptions": {
                    "paths": { "from-a": ["./a-target"] }
                }
            }"#,
        );
        write_file(
            &ws.join("b.json"),
            r#"{
                "compilerOptions": {
                    "paths": { "from-b": ["./b-target"] }
                }
            }"#,
        );

        let user_ts = ws.join("tsconfig.json");
        write_file(&user_ts, r#"{ "extends": ["./a.json", "./b.json"] }"#);

        let layout = CacheLayout::for_workspace(&ws);
        let overlay = build(&layout, &user_ts, &[], &[]);

        let paths = overlay["compilerOptions"]["paths"].as_object().unwrap();
        // Both entries flow through to the overlay — BFS per-pattern
        // first-wins means patterns declared in either base survive.
        assert!(
            paths.contains_key("from-a"),
            "from-a missing from paths; got {:?}",
            paths.keys().collect::<Vec<_>>(),
        );
        assert!(
            paths.contains_key("from-b"),
            "from-b missing from paths; got {:?}",
            paths.keys().collect::<Vec<_>>(),
        );
    }

    #[test]
    fn build_overlay_flattens_sibling_refs_on_solution_redirect() {
        // Solution root at /root with references to src/console (the
        // redirect target) and src/services (a sibling). Services has
        // its own include/exclude/paths. Overlay built around console
        // should carry the services' include/exclude/paths so
        // transitive imports into services don't fire tsgo's "File
        // not listed within project".
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        // The redirect target. Declares `../services` in its OWN
        // references[] — that's how the overlay discovers which
        // siblings to flatten. Solution root coordinates via its
        // own references[] (for `tsc -b` ordering) but the overlay
        // follows the sub-project's declared dependencies, not the
        // solution root's (pulling every solution-root sibling
        // would over-include; see overlay.rs comment).
        write_file(
            &root.join("src/console/tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": { "@": ["./src"] }
                },
                "include": ["**/*.ts"],
                "references": [{ "path": "../services" }]
            }"#,
        );

        // The sibling project — referenced from the console config.
        write_file(
            &root.join("src/services/tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": { "~/*": ["./*"] }
                },
                "include": ["**/*.ts"],
                "exclude": ["fixtures/**/*"]
            }"#,
        );

        // Solution root — coordinates via references (mirrors a real
        // monorepo's `tsc -b` wiring).
        write_file(
            &root.join("tsconfig.json"),
            r#"{
                "files": [],
                "references": [
                    { "path": "./src/console" },
                    { "path": "./src/services" }
                ]
            }"#,
        );

        let console_dir = root.join("src/console");
        let console_ts = console_dir.join("tsconfig.json");
        let layout = CacheLayout::for_workspace_with_solution_root(
            &console_dir,
            Some(root.join("tsconfig.json")),
        );
        let overlay = build(&layout, &console_ts, &[], &[]);

        // `include`: the services' `**/*.ts`, anchored at services'
        // project_dir.
        let includes: Vec<&str> = overlay["include"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let services_dir = root.join("src/services");
        let expected_services_include = services_dir.join("**/*.ts").to_string_lossy().into_owned();
        assert!(
            includes.iter().any(|v| *v == expected_services_include),
            "expected sibling-services include {expected_services_include:?}, got {includes:?}",
        );

        // `exclude`: the services' `fixtures/**/*` resolved absolute.
        let excludes: Vec<&str> = overlay["exclude"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let expected_services_exclude = services_dir
            .join("fixtures/**/*")
            .to_string_lossy()
            .into_owned();
        assert!(
            excludes.iter().any(|v| *v == expected_services_exclude),
            "expected sibling-services exclude {expected_services_exclude:?}, got {excludes:?}",
        );

        // `paths`: console's `@` survives + services' `~/*` was
        // merged in (inner-wins per pattern — both declare disjoint
        // keys, both should appear).
        let paths = overlay["compilerOptions"]["paths"].as_object().unwrap();
        assert!(
            paths.contains_key("@"),
            "console's @ missing: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
        assert!(
            paths.contains_key("~/*"),
            "services' ~/* missing: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn build_overlay_skips_self_reference_in_sibling_flatten() {
        // The solution root references the redirect target itself.
        // That reference should be skipped when flattening siblings —
        // the target's own chain already covers it; re-adding via
        // flatten would duplicate includes.
        let tmp = tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();

        write_file(
            &root.join("app/tsconfig.json"),
            r#"{ "include": ["src/**/*.ts"] }"#,
        );
        write_file(
            &root.join("tsconfig.json"),
            r#"{
                "files": [],
                "references": [{ "path": "./app" }]
            }"#,
        );

        let app_dir = root.join("app");
        let layout = CacheLayout::for_workspace_with_solution_root(
            &app_dir,
            Some(root.join("tsconfig.json")),
        );
        let overlay = build(&layout, &app_dir.join("tsconfig.json"), &[], &[]);

        // `include` should contain the app's own pattern EXACTLY
        // once (anchored at app_dir via the chain walk).
        let includes: Vec<&str> = overlay["include"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        let own_include = app_dir.join("src/**/*.ts").to_string_lossy().into_owned();
        let matches = includes.iter().filter(|v| **v == own_include).count();
        assert_eq!(
            matches, 1,
            "self-reference should not duplicate the include; got {includes:?}",
        );
    }

    #[test]
    fn split_package_entry_unscoped_bare_name() {
        assert_eq!(split_package_entry("node"), ("node", ""));
    }

    #[test]
    fn split_package_entry_unscoped_subpath() {
        assert_eq!(split_package_entry("vite/client"), ("vite", "client"));
        assert_eq!(split_package_entry("vitest/globals"), ("vitest", "globals"),);
        assert_eq!(
            split_package_entry("swiper/css/navigation"),
            ("swiper", "css/navigation"),
        );
    }

    #[test]
    fn split_package_entry_scoped_bare_name() {
        assert_eq!(split_package_entry("@sveltejs/kit"), ("@sveltejs/kit", ""),);
    }

    #[test]
    fn split_package_entry_scoped_subpath() {
        assert_eq!(
            split_package_entry("@sveltejs/kit/types"),
            ("@sveltejs/kit", "types"),
        );
        assert_eq!(
            split_package_entry("@types/node/fs/promises"),
            ("@types/node", "fs/promises"),
        );
    }

    #[test]
    fn split_package_entry_malformed_scoped_stays_whole() {
        // A bare `@scope` with no slash has no package root to split from.
        // Return it unchanged rather than crash.
        assert_eq!(split_package_entry("@scope"), ("@scope", ""));
    }

    #[test]
    fn is_filesystem_types_entry_picks_relative_and_absolute() {
        assert!(is_filesystem_types_entry("./foo"));
        assert!(is_filesystem_types_entry("../foo/bar.d.ts"));
        assert!(is_filesystem_types_entry("/abs/path/foo.d.ts"));
        assert!(!is_filesystem_types_entry("foo"));
        assert!(!is_filesystem_types_entry("vite/client"));
        assert!(!is_filesystem_types_entry("@scope/pkg/sub"));
    }

    #[test]
    fn is_resolvable_types_entry_keeps_installed_package_subpath() {
        // Repro of the real bug: a tsconfig declares `types: ["vite/client"]`
        // and `node_modules/vite/package.json` exists. Pre-fix the entry
        // was classified as a relative filesystem path, not found on
        // disk, and silently dropped — which erased the ambient types
        // user code depends on (`import.meta.env`, CSS module imports).
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp.path().join("node_modules").join("vite");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("package.json"), "{}").unwrap();
        assert!(is_resolvable_types_entry("vite/client", tmp.path()));
        assert!(is_resolvable_types_entry("vite", tmp.path()));
    }

    #[test]
    fn is_resolvable_types_entry_keeps_installed_scoped_subpath() {
        let tmp = tempfile::tempdir().unwrap();
        let pkg = tmp
            .path()
            .join("node_modules")
            .join("@sveltejs")
            .join("kit");
        std::fs::create_dir_all(&pkg).unwrap();
        std::fs::write(pkg.join("package.json"), "{}").unwrap();
        assert!(is_resolvable_types_entry("@sveltejs/kit/types", tmp.path(),));
    }

    #[test]
    fn is_resolvable_types_entry_drops_uninstalled_package() {
        // The filtering's whole point: SvelteKit writes `types: ["node"]`
        // into `.svelte-kit/tsconfig.json` even when @types/node isn't
        // installed; if we kept it tsgo would fire fatal TS2688 and zero
        // our error count. Same applies to uninstalled subpaths.
        let tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(tmp.path().join("node_modules")).unwrap();
        assert!(!is_resolvable_types_entry("node", tmp.path()));
        assert!(!is_resolvable_types_entry("vite/client", tmp.path()));
    }

    #[test]
    fn is_resolvable_types_entry_keeps_relative_dts() {
        let tmp = tempfile::tempdir().unwrap();
        let dts = tmp.path().join("types.d.ts");
        std::fs::write(&dts, "").unwrap();
        assert!(is_resolvable_types_entry("./types.d.ts", tmp.path()));
        // Extensionless form also accepted (tsgo appends .d.ts).
        assert!(is_resolvable_types_entry("./types", tmp.path()));
    }

    #[test]
    fn is_resolvable_types_entry_drops_missing_relative_file() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(!is_resolvable_types_entry("./does-not-exist", tmp.path()));
    }
}
