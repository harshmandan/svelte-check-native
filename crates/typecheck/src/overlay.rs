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
    // Whatever the user's extends chain declared.
    for entry in collect_user_root_dirs(user_tsconfig) {
        push_root(entry.as_path(), &mut root_dirs, &mut seen);
    }

    // Path aliases: collect every entry the user defined in their
    // extends chain and prepend a cache-mirror candidate. TS resolves
    // each pattern's value-list in order, so a path-mapped import like
    // `$lib/foo/Bar.svelte.ts` first tries our generated overlay file
    // and falls back to the source location if not found. Without this,
    // path-mapped Svelte imports skip rootDirs entirely and never reach
    // our overlay (rootDirs only kicks in for raw relative paths).
    let mut paths_map: serde_json::Map<String, Value> = serde_json::Map::new();
    for (pattern, values) in collect_user_paths(user_tsconfig) {
        let mut merged: Vec<String> = Vec::new();
        for v in &values {
            // The cache mirror sits at <overlay>/svelte/<same-relative-segment>.
            // The tail after the workspace root in `v` is what we mirror.
            let mirrored = mirror_into_overlay(layout, v);
            if let Some(m) = mirrored {
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
    // `allowImportingTsExtensions` lets emit rewrite
    // `import './X.svelte'` → `import './X.svelte.ts'` so the import
    // lands on our generated overlay file rather than resolving to the
    // `*.svelte` ambient declaration shipped with the `svelte` package.
    compiler_options.insert("allowImportingTsExtensions".into(), json!(true));
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
    if let Some(types) = collect_user_types(user_tsconfig) {
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
    let user_includes = collect_user_includes(user_tsconfig);

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
    let mut excludes: Vec<String> = collect_user_excludes(user_tsconfig);
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

/// Walk the user tsconfig's `extends` chain and collect every `paths`
/// entry, resolving relative paths-target values against the declaring
/// tsconfig's directory (or its `baseUrl` when set). Inner configs
/// override outer (as TS does).
///
/// `extends` may be a single string OR an array of strings (TS 5.0+).
/// Arrays are walked left-to-right; later entries override earlier
/// ones for conflicting keys, matching TS semantics. Combined with
/// "inner wins over outer," the precedence is: the first config to
/// declare a key in a depth-first inner-first walk wins.
fn collect_user_paths(tsconfig: &Path) -> Vec<(String, Vec<String>)> {
    use std::collections::HashMap;
    let mut accumulated: HashMap<String, Vec<String>> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    // FIFO queue rather than single-pointer so array-extends work.
    // Cap iterations to keep pathological cycles from looping.
    let mut queue: std::collections::VecDeque<PathBuf> =
        std::collections::VecDeque::from([tsconfig.to_path_buf()]);
    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut hops = 0usize;
    while let Some(path) = queue.pop_front() {
        hops += 1;
        if hops > 32 {
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = json5::from_str::<Value>(&content) else {
            continue;
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        let opts = json.get("compilerOptions");
        // baseUrl resolves relative paths inside `paths` values.
        let base_url = opts
            .and_then(|c| c.get("baseUrl"))
            .and_then(Value::as_str)
            .map(|b| {
                if Path::new(b).is_absolute() {
                    PathBuf::from(b)
                } else {
                    dir.join(b)
                }
            })
            .unwrap_or_else(|| dir.to_path_buf());
        if let Some(p) = opts.and_then(|c| c.get("paths")).and_then(Value::as_object) {
            for (pattern, values) in p {
                if accumulated.contains_key(pattern) {
                    // Inner (closer-to-root) wins. Skip outer entries.
                    continue;
                }
                let arr = match values.as_array() {
                    Some(a) => a,
                    None => continue,
                };
                let mut resolved: Vec<String> = Vec::new();
                for v in arr {
                    let Some(s) = v.as_str() else { continue };
                    let abs = if Path::new(s).is_absolute() {
                        PathBuf::from(s)
                    } else {
                        base_url.join(s)
                    };
                    resolved.push(normalize(&abs).to_string_lossy().into_owned());
                }
                if !resolved.is_empty() {
                    order.push(pattern.clone());
                    accumulated.insert(pattern.clone(), resolved);
                }
            }
        }
        for ext in resolve_extends(&json, dir) {
            queue.push_back(ext);
        }
    }
    order
        .into_iter()
        .filter_map(|k| accumulated.remove(&k).map(|v| (k, v)))
        .collect()
}

/// Read an `extends` field that may be a single string or an array of
/// strings, returning the resolved absolute path(s) in declaration
/// order. Returns empty if `extends` is absent or malformed.
fn resolve_extends(json: &Value, dir: &Path) -> Vec<PathBuf> {
    let Some(extends) = json.get("extends") else {
        return Vec::new();
    };
    let resolve_one = |s: &str| -> PathBuf {
        if Path::new(s).is_absolute() {
            PathBuf::from(s)
        } else {
            dir.join(s)
        }
    };
    if let Some(s) = extends.as_str() {
        return vec![resolve_one(s)];
    }
    if let Some(arr) = extends.as_array() {
        return arr
            .iter()
            .filter_map(|v| v.as_str().map(resolve_one))
            .collect();
    }
    Vec::new()
}

/// Walk the user tsconfig's `extends` chain (depth-limited; tsconfig
/// chains shouldn't realistically exceed a handful of links) and return
/// every `rootDirs` entry resolved to an absolute path.
///
/// Unreadable, malformed, or missing files cause the walk to stop — we
/// don't care about producing perfect output, only about preserving as
/// much of the user's intent as we can. The overlay still works (just
/// with fewer virtual roots merged) when this returns empty.
fn collect_user_root_dirs(tsconfig: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut queue: std::collections::VecDeque<PathBuf> =
        std::collections::VecDeque::from([tsconfig.to_path_buf()]);
    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut hops = 0usize;
    while let Some(path) = queue.pop_front() {
        hops += 1;
        if hops > 32 {
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = json5::from_str::<Value>(&content) else {
            continue;
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        if let Some(rd) = json
            .get("compilerOptions")
            .and_then(|c| c.get("rootDirs"))
            .and_then(|v| v.as_array())
        {
            for entry in rd {
                if let Some(s) = entry.as_str() {
                    let resolved = if Path::new(s).is_absolute() {
                        PathBuf::from(s)
                    } else {
                        dir.join(s)
                    };
                    out.push(normalize(&resolved));
                }
            }
        }
        for ext in resolve_extends(&json, dir) {
            queue.push_back(ext);
        }
    }
    out
}

/// Walk the user tsconfig's `extends` chain and collect every `include`
/// pattern, resolved to an absolute path-glob. Only the FIRST tsconfig
/// in the chain that declares `include` wins, matching TS behavior:
/// `include` in an inner config overrides any inherited from `extends`.
///
/// Patterns matching `**/*.svelte` (or anything that ends in
/// `.svelte`) are dropped — tsgo can't parse raw .svelte and the
/// .svelte content is already covered by our generated overlay files
/// in the `files` array. Keeping `.svelte` patterns would either fire
/// "file is not a TS file" errors or get silently ignored depending on
/// tsgo build; either way they add nothing useful.
fn collect_user_includes(tsconfig: &Path) -> Vec<String> {
    collect_user_patterns(tsconfig, "include", true)
}

/// Walks the extends chain and returns the user's `exclude` patterns
/// as absolute path globs. Same shape as [`collect_user_includes`] —
/// returned when we need to carry them forward into our own overlay
/// `exclude` (e.g. when we also want to exclude Kit-file source
/// originals). tsconfig `exclude` is REPLACED not merged when a child
/// defines it, so we must union the user's patterns into ours
/// explicitly.
fn collect_user_excludes(tsconfig: &Path) -> Vec<String> {
    collect_user_patterns(tsconfig, "exclude", false)
}

/// Shared backbone for `include` / `exclude` resolution. Walks the
/// extends chain, returns the first config that declares the field.
/// `drop_svelte_only` filters out patterns that only matched raw
/// `.svelte` files — valid for `include` where our generated
/// `.svelte.ts` overlays cover the same surface, harmful for
/// `exclude` where dropping a pattern opens a hole.
fn collect_user_patterns(tsconfig: &Path, field: &str, drop_svelte_only: bool) -> Vec<String> {
    let mut queue: std::collections::VecDeque<PathBuf> =
        std::collections::VecDeque::from([tsconfig.to_path_buf()]);
    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut hops = 0usize;
    while let Some(path) = queue.pop_front() {
        hops += 1;
        if hops > 32 {
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = json5::from_str::<Value>(&content) else {
            continue;
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        if let Some(arr) = json.get(field).and_then(|v| v.as_array()) {
            // First config to declare the field wins (TS semantics).
            let mut out: Vec<String> = Vec::new();
            for entry in arr {
                let Some(s) = entry.as_str() else { continue };
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
        for ext in resolve_extends(&json, dir) {
            queue.push_back(ext);
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

/// Walk the user tsconfig's `extends` chain to find the first `types`
/// entry, then filter it: package-name entries pass through, relative
/// path entries are kept only if they actually exist on disk. Returns
/// `Some(filtered_list)` when the chain declared `types`, `None`
/// otherwise (so we don't override an unset value).
///
/// Filtering matters because tsgo treats a non-resolvable `types`
/// entry as a fatal TS2688 — once that fires, downstream files in the
/// program go un-diagnosed. Projects that auto-generate a .d.ts (e.g.
/// unplugin-auto-import → `.generated/types/auto-imports.d.ts`) hit
/// this pattern any time the user has a fresh clone or a stale build.
fn collect_user_types(tsconfig: &Path) -> Option<Vec<String>> {
    let mut queue: std::collections::VecDeque<PathBuf> =
        std::collections::VecDeque::from([tsconfig.to_path_buf()]);
    let mut visited: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    let mut hops = 0usize;
    while let Some(path) = queue.pop_front() {
        hops += 1;
        if hops > 32 {
            break;
        }
        if !visited.insert(path.clone()) {
            continue;
        }
        let Ok(content) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(json) = json5::from_str::<Value>(&content) else {
            continue;
        };
        let dir = path.parent().unwrap_or(Path::new(""));
        if let Some(arr) = json
            .get("compilerOptions")
            .and_then(|c| c.get("types"))
            .and_then(|v| v.as_array())
        {
            // Inner config wins (TS semantics).
            let mut out: Vec<String> = Vec::new();
            for entry in arr {
                let Some(s) = entry.as_str() else { continue };
                if is_resolvable_types_entry(s, dir) {
                    out.push(s.to_string());
                }
            }
            return Some(out);
        }
        for ext in resolve_extends(&json, dir) {
            queue.push_back(ext);
        }
    }
    None
}

/// True when a `types` entry will resolve under tsgo's lookup rules.
///
/// **Package-name entries** (e.g. `"node"`, `"@types/foo"`, `"foo"`)
/// are resolved through the workspace's `node_modules` chain. We walk
/// up from the declaring tsconfig's directory looking for either
/// `node_modules/@types/<name>/package.json` (the conventional types
/// package layout) or `node_modules/<name>/package.json` (a runtime
/// package that ships its own .d.ts files). Either match means tsgo
/// will find it; otherwise we drop the entry. This matters because
/// SvelteKit's auto-generated `.svelte-kit/tsconfig.json` declares
/// `types: ["node"]` even when the host project doesn't actually
/// depend on `@types/node` — without filtering, tsgo treats the
/// missing entry as fatal TS2688 and stops emitting diagnostics for
/// the entire program.
///
/// **Relative path entries** (start with `.` or `/`, or contain a
/// path separator and aren't scoped) must point at an existing file.
/// tsgo's `types` lookup for relative entries does NOT add `.d.ts`
/// automatically when the path already includes an extension, so we
/// test the literal path first and `<path>.d.ts` as a fallback.
fn is_resolvable_types_entry(entry: &str, declaring_dir: &Path) -> bool {
    let looks_relative = entry.starts_with('.')
        || entry.starts_with('/')
        || (entry.contains('/') && !entry.starts_with('@'));
    if !looks_relative {
        return package_types_entry_resolves(entry, declaring_dir);
    }
    let candidate = if Path::new(entry).is_absolute() {
        PathBuf::from(entry)
    } else {
        declaring_dir.join(entry)
    };
    if candidate.is_file() {
        return true;
    }
    // `types: ["./foo"]` may resolve as `foo.d.ts`.
    let with_dts = candidate.with_extension(format!(
        "{}.d.ts",
        candidate.extension().and_then(|e| e.to_str()).unwrap_or(""),
    ));
    if with_dts.is_file() {
        return true;
    }
    // Plain extensionless input: try `<entry>.d.ts`.
    let mut as_dts = candidate.clone();
    as_dts.as_mut_os_string().push(".d.ts");
    as_dts.is_file()
}

/// True when a bare-name `types` entry resolves to either an `@types`
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
}
