//! Load a tsconfig with full `extends` chain resolution + `${configDir}`
//! substitution + merging.
//!
//! Two views are offered. [`load`] is the fully-merged convenience view: it
//! collapses the whole `extends` chain into one [`TsConfigFile`] with TS's
//! replace-on-child semantics, as if the user had written one giant flat
//! config. [`load_chain`] is the path-aware production path the overlay
//! builder and CLI use: it returns each file in the chain unmerged, so
//! callers can rebase relative paths against each file's own directory.
//!
//! ### Resolution
//!
//! - Relative `extends` (`./`, `../`, or absolute path): resolved against the
//!   directory of the config that wrote it. If the path has no extension,
//!   tries `.json` then the bare path.
//! - Package `extends` (e.g. `@tsconfig/svelte`, `@tsconfig/svelte/tsconfig.json`,
//!   `my-tsconfig`): node-style walk up from the current config's dir looking
//!   for `node_modules/<pkg>`. A package.json `exports` map, when present, is
//!   the exclusive way into the package (mirroring TS's NodeNext config
//!   lookup). Exports-less packages resolve bare names through the
//!   package.json `"tsconfig"` field, defaulting to `tsconfig.json`, and
//!   subpaths through the literal file layout with `.json` appended.
//!
//! ### `${configDir}` substitution
//!
//! The placeholder expands to the absolute path of the directory containing
//! the ROOT config being loaded (the entry tsconfig), never the extended
//! file's own dir — TypeScript 5.5 semantics. So if `base.json` has
//! `"baseUrl": "${configDir}/src"` and the user's `tsconfig.json` extends it,
//! `${configDir}` resolves to the *user tsconfig's* dir. Implemented by
//! threading the entry dir through the extends chain (see `entry_dir` below).
//!
//! ### Merge rules (match TypeScript's behavior)
//!
//! - `compilerOptions`: shallow merge — child's keys override parent's;
//!   parent's keys absent in child are inherited. `raw` is also shallow-merged
//!   so unknown fields inherit the same way.
//! - `paths`: REPLACED entirely if child specifies it (not per-key merge).
//! - `rootDirs`: REPLACED if child specifies non-empty.
//! - `typeRoots` / `types`: REPLACED if child specifies (even empty).
//! - `include`, `exclude`, `files`: REPLACED if child specifies.
//! - `references`: NOT inherited — TS reads it only from the config
//!   being loaded, never from an extended parent. Always taken from the
//!   leaf (even when empty).
//! - Final config's `path` is set to the entry file (the leaf of the chain).

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::parse::{ParseError, parse_file};
use super::version_range::{Version, VersionRange};
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
    let mut merged = load_recursive(entry.as_ref(), &mut seen)?;
    // `${configDir}` resolves against the ENTRY config's directory for the
    // whole merged result — run once, at the end, so an inherited
    // placeholder from a different-directory base config resolves into the
    // consuming project (matches TS). `merged.path` is the entry's
    // canonical path, so `config_dir()` is the entry dir.
    let entry_dir = merged.config_dir().to_path_buf();
    substitute_config_dir(&mut merged, &entry_dir);
    Ok(merged)
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
/// used to do. A missing entry file still surfaces via [`LoadError`]
/// (the entry must canonicalize); everything reached through `extends`,
/// including a malformed file, is skipped rather than raised.
pub fn load_chain(entry: impl AsRef<Path>) -> Result<Vec<TsConfigFile>, LoadError> {
    use std::collections::VecDeque;

    let entry_canon = dunce::canonicalize(entry.as_ref()).map_err(|source| LoadError::Io {
        path: entry.as_ref().to_path_buf(),
        source,
    })?;

    // `${configDir}` in ANY file of the chain resolves against the ENTRY
    // config's directory (TS semantics), not each file's own dir.
    let entry_dir = entry_canon
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();

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
        substitute_config_dir(&mut file, &entry_dir);

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

/// Find the config in `chain` (as returned by [`load_chain`]) whose
/// declaration of a top-level pattern field (`include` / `exclude` /
/// `files`) wins under TypeScript's `extends` precedence, and return it
/// together with the winning patterns.
///
/// Precedence is the same replace-on-child rule [`load`]'s `merge_into`
/// applies: the entry (leaf) wins whenever it declares the field — an
/// explicit empty array counts as a declaration — otherwise the field
/// comes from the `extends` list where LATER entries override earlier
/// ones, each entry resolved recursively through its own parents.
///
/// Returning the declaring [`TsConfigFile`] (not just the patterns)
/// matters because TS resolves `include`/`exclude`/`files` relative to
/// the directory of the config file that declares them — callers rebase
/// against `winner.config_dir()`.
///
/// Returns `None` when no config in the chain declares the field.
pub fn winning_patterns<'a, F>(
    chain: &'a [TsConfigFile],
    get: F,
) -> Option<(&'a TsConfigFile, &'a [String])>
where
    F: Fn(&'a TsConfigFile) -> Option<&'a [String]>,
{
    winning_field(chain, get)
}

/// Like [`winning_patterns`] but for any field, not just pattern lists.
///
/// Same precedence walk: the entry config wins whenever it declares the
/// field, otherwise LATER `extends` entries beat earlier ones, each
/// resolved recursively through its own parents.
///
/// Scalar `compilerOptions` need this too. Reading them by scanning the
/// chain in BFS order — which is what several callers used to do — picks
/// whichever config appears first in the load order, not the one TS
/// would let win. For `extends: ["./a.json", "./b.json"]` where `a`
/// declares a field directly and `b` inherits it from its own parent,
/// BFS yields `a` and TypeScript yields `b`'s parent.
pub fn winning_field<'a, T, F>(chain: &'a [TsConfigFile], get: F) -> Option<(&'a TsConfigFile, T)>
where
    F: Fn(&'a TsConfigFile) -> Option<T>,
{
    let entry = chain.first()?;
    let mut visited: HashSet<&Path> = HashSet::new();
    winning_field_from(chain, entry, &get, &mut visited)
}

fn winning_field_from<'a, T, F>(
    chain: &'a [TsConfigFile],
    file: &'a TsConfigFile,
    get: &F,
    visited: &mut HashSet<&'a Path>,
) -> Option<(&'a TsConfigFile, T)>
where
    F: Fn(&'a TsConfigFile) -> Option<T>,
{
    if !visited.insert(file.path.as_path()) {
        // Extends cycle — load_chain already de-duplicated the files, so
        // revisiting can only happen through a cyclic reference; stop.
        return None;
    }
    if let Some(value) = get(file) {
        return Some((file, value));
    }
    // Later extends entries override earlier ones (TS array-extends), so
    // probe the parents in reverse: the first hit is the winner.
    let parent_dir = file
        .path
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_default();
    for ext_ref in file.extends.iter().rev() {
        let Ok(resolved) = resolve_extends(ext_ref, &parent_dir) else {
            continue;
        };
        let canon = dunce::canonicalize(&resolved).unwrap_or(resolved);
        let Some(parent) = chain.iter().find(|f| f.path == canon) else {
            continue;
        };
        if let Some(hit) = winning_field_from(chain, parent, get, visited) {
            return Some(hit);
        }
    }
    None
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
    // one. `${configDir}` substitution is deliberately NOT done here —
    // placeholders are left literal through the extends merge and resolved
    // once against the ENTRY dir in `load` (TS semantics; see
    // `substitute_config_dir`).
    file.path = canonical.clone();

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

/// Substitute every `${configDir}` placeholder against `entry_dir` — the
/// directory of the ROOT config the user is compiling, NOT the directory
/// of whichever file in the extends chain wrote the placeholder. This is
/// TypeScript's design intent: a shared base config (e.g. in
/// `node_modules` or a sibling `configs/`) writes `"baseUrl":
/// "${configDir}/src"` and it must resolve into the CONSUMING project.
/// (`parseJsonConfigFileContentWorker` runs the substitution once at the
/// end on the fully-merged options with `basePath` = the root config's
/// dir; the extends merge leaves the placeholder literal so the final
/// substitution wins.)
fn substitute_config_dir(file: &mut TsConfigFile, entry_dir: &Path) {
    let dir = entry_dir.to_string_lossy().into_owned();

    // TypeScript honours the placeholder only when it STARTS the value —
    // `"./x/${configDir}/y"` is left literal. Substituting anywhere would
    // invent a meaning the compiler doesn't give it.
    let sub = |s: &mut String| {
        if let Some(rest) = s.strip_prefix(CONFIG_DIR) {
            let mut out = dir.clone();
            out.push_str(rest);
            *s = out;
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
    if let Some(p) = co.paths.as_mut() {
        for vs in p.values_mut() {
            sub_vec(vs);
        }
    }
    sub_opt(&mut co.declaration_dir);
    sub_opt_vec(&mut co.type_roots);
    sub_opt_vec(&mut co.types);

    // Walk unknown compilerOptions values too — users can put ${configDir} in
    // anything and we have to pass it through correctly. Record which keys
    // were touched: the overlay must re-emit exactly those, because the
    // compiler would otherwise re-resolve the placeholder against the
    // overlay's own directory. See `TsConfigFile::config_dir_keys`.
    for (key, value) in co.raw.iter_mut() {
        if walk_value(value, &dir) {
            file.config_dir_keys.push(key.clone());
        }
    }

    sub_opt_vec(&mut file.include);
    sub_opt_vec(&mut file.exclude);
    sub_opt_vec(&mut file.files);
    // NOT substituted in `references[].path`: TypeScript leaves the
    // placeholder literal there, and `extends` is resolved before this
    // pass runs, so both match the compiler by construction.
}

const CONFIG_DIR: &str = "${configDir}";

/// Substitute `${configDir}` throughout `v`. Returns true if anything
/// changed.
fn walk_value(v: &mut Value, dir: &str) -> bool {
    match v {
        Value::String(s) => {
            if let Some(rest) = s.strip_prefix(CONFIG_DIR) {
                let mut out = dir.to_string();
                out.push_str(rest);
                *s = out;
                return true;
            }
            false
        }
        Value::Array(arr) => {
            let mut hit = false;
            for x in arr {
                hit |= walk_value(x, dir);
            }
            hit
        }
        Value::Object(map) => {
            let mut hit = false;
            for x in map.values_mut() {
                hit |= walk_value(x, dir);
            }
            hit
        }
        _ => false,
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

    // Try the literal path first (covers `./base.json` and the rare
    // extensionless file). TypeScript then APPENDS `.json` — note
    // append, not replace — to any reference that doesn't already end
    // in `.json`: `./tsconfig.base` resolves `tsconfig.base.json`, not
    // `tsconfig.json`. We key off the literal `.json` suffix like TS,
    // NOT `Path::extension()`: a dotted basename such as
    // `tsconfig.base` has `extension() == Some("base")`, so an
    // extension-presence check would skip the append and fail to find
    // `tsconfig.base.json` (a common monorepo convention).
    // `with_extension("json")` is also wrong here — it would *replace*,
    // turning `tsconfig.base` into `tsconfig.json`.
    if candidate.is_file() {
        return Ok(candidate);
    }
    if !reference.ends_with(".json") {
        let mut with_json = candidate.into_os_string();
        with_json.push(".json");
        let with_json = PathBuf::from(with_json);
        if with_json.is_file() {
            return Ok(with_json);
        }
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
                package_subpath_config(&pkg_root, sp)
            } else {
                resolve_package_root_config(&pkg_root)
            };
            if let Some(resolved) = resolved
                && resolved.is_file()
            {
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

/// Resolve a package extends that names a subpath (`<pkg>/<subpath>`) to
/// a file inside the package.
///
/// TypeScript resolves such a reference as a module with the `.json`
/// extension, so an extensionless subpath picks up `.json` — exactly the
/// same append-don't-replace rule the relative branch applies. The
/// literal path is tried first so a reference that already spells out
/// `.json` (or names an extensionless file) still resolves.
///
/// This matters for SvelteKit 3, whose generated config lives at
/// `node_modules/$app/tsconfig.json` and is referenced as
/// `"extends": "$app/tsconfig"`. Without the append the chain breaks
/// silently and the project loses the generated `rootDirs` — which is
/// what makes `./$types` resolve from a route file.
fn package_subpath_config(pkg_root: &Path, subpath: &str) -> Option<PathBuf> {
    // `exports` first, when the package declares one. A package that maps
    // `"./base"` to `"./src/base.json"` is resolved by TypeScript through
    // that map; probing the literal layout instead finds nothing, and
    // `load_chain` skips a base it cannot resolve — so the consumer
    // silently loses everything that base declared.
    //
    // And the map is exclusive, per Node's exports semantics: when a
    // package declares `exports`, a subpath the map doesn't cover is
    // unreachable — no falling back to the literal file layout. The
    // literal probe applies only to packages with no `exports` at all.
    match package_subpath_export(pkg_root, subpath) {
        SubpathExport::Resolved(target) => return Some(pkg_root.join(target)),
        SubpathExport::NotExported => return None,
        SubpathExport::NoExports => {}
    }
    let literal = pkg_root.join(subpath);
    if literal.is_file() || subpath.ends_with(".json") {
        return Some(literal);
    }
    let mut with_json = literal.into_os_string();
    with_json.push(".json");
    Some(PathBuf::from(with_json))
}

/// Outcome of consulting a package's `exports` map for a subpath.
enum SubpathExport {
    /// No `package.json`, unparseable JSON, or no `exports` field —
    /// legacy file-layout probing applies.
    NoExports,
    /// `exports` maps the subpath to this package-relative target.
    Resolved(String),
    /// `exports` is present but does not export the subpath (or its
    /// target resolves to nothing usable). Resolution through this
    /// package must fail — `exports` is exclusive.
    NotExported,
}

/// Consult the package's `exports` map for `./<subpath>`.
///
/// Mirrors TS's `loadModuleFromExportsOrImports` under config lookup:
/// an exact subpath key wins; otherwise single-`*` pattern keys and
/// trailing-`/` directory keys are tried in `comparePatternKeys` order
/// (longer base first — a pattern key's base runs through its `*`, a
/// directory key's through its trailing `/` — pattern keys before
/// directory keys on a tie, longer key first among equal-base pattern
/// keys). The FIRST key that matches is committed to: a target that
/// then fails to load fails the whole lookup rather than falling
/// through to a lesser key. Fall-through exists only WITHIN a target,
/// across conditions and array entries.
fn package_subpath_export(pkg_root: &Path, subpath: &str) -> SubpathExport {
    let Ok(contents) = std::fs::read_to_string(pkg_root.join("package.json")) else {
        return SubpathExport::NoExports;
    };
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&contents) else {
        return SubpathExport::NoExports;
    };
    let Some(exports) = obj.get("exports") else {
        return SubpathExport::NoExports;
    };
    // The sugar forms (`"exports": "./x.json"`, or an object with no
    // dot-prefixed keys, i.e. a bare condition map) export only `"."` —
    // every subpath is unexported.
    let Some(map) = exports
        .as_object()
        .filter(|m| m.keys().any(|k| k.starts_with('.')))
    else {
        return SubpathExport::NotExported;
    };

    let commit = |target: Option<String>| match target {
        Some(target) => SubpathExport::Resolved(target),
        None => SubpathExport::NotExported,
    };

    // Normalize to the `./` prefix `exports` keys carry.
    let key = format!("./{}", subpath.trim_start_matches("./"));
    if !key.contains('*')
        && !key.ends_with('/')
        && let Some(entry) = map.get(key.as_str())
    {
        return commit(resolve_export_target(pkg_root, entry, "", false));
    }

    let mut keys: Vec<&str> = map
        .keys()
        .map(String::as_str)
        .filter(|k| has_one_asterisk(k) || k.ends_with('/'))
        .collect();
    keys.sort_by(|a, b| compare_pattern_keys(a, b));
    for k in keys {
        let entry = &map[k];
        if let Some(star) = k.find('*') {
            if !k.ends_with('*') {
                // `prefix*suffix` trailer pattern.
                let (prefix, suffix) = (&k[..star], &k[star + 1..]);
                if key.starts_with(prefix) && key.ends_with(suffix) {
                    if key.len() < prefix.len() + suffix.len() {
                        // Prefix and suffix overlap inside the key. TS
                        // still commits to this key, and the capture
                        // its unguarded substring arithmetic produces
                        // can never resolve — committing to failure is
                        // the faithful outcome.
                        return SubpathExport::NotExported;
                    }
                    let capture = &key[prefix.len()..key.len() - suffix.len()];
                    return commit(resolve_export_target(pkg_root, entry, capture, true));
                }
            } else if let Some(capture) = key.strip_prefix(&k[..k.len() - 1]) {
                return commit(resolve_export_target(pkg_root, entry, capture, true));
            }
        } else if let Some(rest) = key.strip_prefix(k) {
            // Directory key (trailing `/`): the remainder appends to
            // the target, which must itself end in `/`.
            return commit(resolve_export_target(pkg_root, entry, rest, false));
        }
    }
    SubpathExport::NotExported
}

/// A key participates in pattern matching with exactly one `*` (TS
/// `hasOneAsterisk`); more are invalid and the key is ignored.
fn has_one_asterisk(key: &str) -> bool {
    let mut stars = key.match_indices('*');
    stars.next().is_some() && stars.next().is_none()
}

/// TS `comparePatternKeys`: sort pattern/directory keys so the most
/// specific comes first. Base length is measured through the `*` for
/// pattern keys and the whole key for directory keys.
fn compare_pattern_keys(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let ap = a.find('*');
    let bp = b.find('*');
    let base_a = ap.map_or(a.len(), |i| i + 1);
    let base_b = bp.map_or(b.len(), |i| i + 1);
    base_b.cmp(&base_a).then_with(|| match (ap, bp) {
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        _ => b.len().cmp(&a.len()),
    })
}

/// Resolve a bare package extends (no subpath) to the package's config
/// file, reading the package's `package.json` once.
///
/// When the package declares `exports`, the `"."` entry is the ONLY way
/// in: a `"."` that is absent or fails to resolve fails the package
/// (verified against tsgo — no fallback to the `tsconfig` field or to
/// `tsconfig.json`). Only exports-less packages fall through to the
/// legacy `"tsconfig"`-field-then-`tsconfig.json` order.
fn resolve_package_root_config(pkg_root: &Path) -> Option<PathBuf> {
    let default = || Some(pkg_root.join("tsconfig.json"));
    let Ok(contents) = std::fs::read_to_string(pkg_root.join("package.json")) else {
        return default();
    };
    let Ok(Value::Object(obj)) = serde_json::from_str::<Value>(&contents) else {
        return default();
    };

    if let Some(exports) = obj.get("exports") {
        return resolve_dot_export(pkg_root, exports).map(|target| pkg_root.join(target));
    }

    if let Some(Value::String(ts)) = obj.get("tsconfig") {
        return Some(pkg_root.join(ts));
    }
    default()
}

/// Resolve the `"."` entry of a package.json `exports` value. Handles
/// the bare-string form (`"exports": "./tsconfig.json"`), the
/// `{ ".": <target> }` subpath form, and bare condition objects.
fn resolve_dot_export(pkg_root: &Path, exports: &Value) -> Option<String> {
    // A subpath map (any key starting with '.') exports "." only via a
    // literal "." key; a bare string or condition map IS the "." target.
    let dot = match exports.as_object() {
        Some(map) if map.keys().any(|k| k.starts_with('.')) => map.get(".")?,
        _ => exports,
    };
    resolve_export_target(pkg_root, dot, "", false)
}

/// The conditions active for tsconfig-`extends` resolution. TypeScript
/// resolves the reference as a CommonJS-style module, so `require` and
/// `node` apply (plus `types`); `default` always matches, and versioned
/// `types@<range>` keys match when the range admits the engine version.
const ACTIVE_CONDITIONS: [&str; 3] = ["types", "require", "node"];

/// The engine version versioned `types@<range>` condition keys are
/// evaluated against (TS `isApplicableVersionedTypesKey` tests them
/// with the compiler's own version). Engine discovery only ever selects
/// a TypeScript-7-family compiler — stable `typescript` 7+ or the tsgo
/// native preview — so the version is pinned here rather than plumbed
/// from discovery; a range would have to discriminate WITHIN the 7.x
/// line (or against a preview's prerelease tag) to observe the
/// difference.
const ENGINE_VERSION: Version = Version::new(7, 0, 0);

/// TS `isApplicableVersionedTypesKey`: a `types@<range>` condition key
/// matches when the range parses and admits the engine version. The
/// eligibility gate — the active condition set containing `types` — is
/// satisfied by construction here (see [`ACTIVE_CONDITIONS`]).
fn is_applicable_versioned_types_key(key: &str) -> bool {
    let Some(spec) = key.strip_prefix("types@") else {
        return false;
    };
    match VersionRange::try_parse(spec) {
        Some(range) => range.test(&ENGINE_VERSION),
        None => false,
    }
}

/// Resolve one `exports` target for a subpath, mirroring TS's
/// `loadModuleFromTargetExportOrImport` under config lookup: only a
/// target inside the package — starting `./`, with no `..` / `.` /
/// `node_modules` segments in the target or the capture — that names
/// an EXISTING `.json` file resolves. Anything else fails the branch,
/// and a failed branch falls through to the next condition or array
/// entry (never to another subpath key; the caller commits per key).
///
/// Condition objects are walked in OBJECT KEY ORDER — the order the
/// package author wrote them — taking each key in the active condition
/// set in turn until one resolves. Nested condition objects resolve
/// recursively; array targets take the first entry that resolves.
///
/// `pattern` distinguishes `*` substitution (`subpath` is the capture,
/// replacing every `*` in the target) from directory-key append
/// (`subpath` is the remainder and the target must end in `/`).
fn resolve_export_target(
    pkg_root: &Path,
    target: &Value,
    subpath: &str,
    pattern: bool,
) -> Option<String> {
    match target {
        Value::String(s) => {
            if !pattern && !subpath.is_empty() && !s.ends_with('/') {
                return None;
            }
            if !s.starts_with("./") {
                return None;
            }
            let invalid = |seg: &str| matches!(seg, ".." | "." | "node_modules");
            if s[2..].split('/').any(invalid) || subpath.split('/').any(invalid) {
                return None;
            }
            let resolved = if pattern {
                s.replace('*', subpath)
            } else {
                format!("{s}{subpath}")
            };
            (resolved.ends_with(".json") && pkg_root.join(&resolved).is_file()).then_some(resolved)
        }
        Value::Object(conds) => conds.iter().find_map(|(cond, value)| {
            (cond == "default"
                || ACTIVE_CONDITIONS.contains(&cond.as_str())
                || is_applicable_versioned_types_key(cond))
            .then(|| resolve_export_target(pkg_root, value, subpath, pattern))
            .flatten()
        }),
        Value::Array(candidates) => candidates
            .iter()
            .find_map(|candidate| resolve_export_target(pkg_root, candidate, subpath, pattern)),
        _ => None,
    }
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
    // `references` is NOT inherited through `extends` — TypeScript reads
    // it only from the config actually being loaded, never from an
    // extended parent. The leaf is merged last (see load_recursive), so
    // always taking the child's value (even when empty) yields exactly
    // the leaf's references and drops any a parent declared.
    base.references = child.references;
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
    // `paths` is replaced-when-specified (TS never per-key merges); a
    // child's explicit `Some` (even empty `{}`) blanks the parent's.
    if cc.paths.is_some() {
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
    fn load_with_dotted_basename_extends_appends_json() {
        // extends: "./tsconfig.base" must resolve "tsconfig.base.json"
        // (APPEND .json), not "tsconfig.json" (which `with_extension`
        // would wrongly produce). A common monorepo convention.
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("tsconfig.base.json");
        let decoy = tmp.path().join("tsconfig.json");
        let ts = tmp.path().join("app.tsconfig.json");
        write(&base, r#"{ "compilerOptions": { "strict": true } }"#);
        // A `tsconfig.json` decoy with a conflicting value: if the
        // resolver replaced the extension instead of appending, it would
        // pick this up and `strict` would be false.
        write(&decoy, r#"{ "compilerOptions": { "strict": false } }"#);
        write(&ts, r#"{ "extends": "./tsconfig.base" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    #[test]
    fn references_not_inherited_through_extends() {
        // TS reads `references` only from the config being loaded, never
        // from an extended parent.
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "references": [{ "path": "./packages/a" }] }"#);
        write(&ts, r#"{ "extends": "./base.json" }"#);

        let cfg = load(&ts).unwrap();
        assert!(
            cfg.references.is_empty(),
            "references from an extended parent must not be inherited, got {:?}",
            cfg.references
        );
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
    fn config_dir_substitution_in_parent_uses_entry_dir() {
        let tmp = tempdir().unwrap();
        let base_dir = tmp.path().join("configs");
        let child_dir = tmp.path().join("project");
        fs::create_dir_all(&base_dir).unwrap();
        fs::create_dir_all(&child_dir).unwrap();

        let base = base_dir.join("base.json");
        let ts = child_dir.join("tsconfig.json");

        // A base config's ${configDir} resolves to the ENTRY (child) dir,
        // NOT the base's own dir — TS semantics: a shared base resolves
        // into the consuming project.
        write(
            &base,
            r#"{ "compilerOptions": { "rootDirs": ["${configDir}/src"] } }"#,
        );
        write(&ts, r#"{ "extends": "../configs/base.json" }"#);

        let cfg = load(&ts).unwrap();
        let expected = child_dir.canonicalize().unwrap().join("src");
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
        let paths = cfg.compiler_options.paths.as_ref().unwrap();
        assert_eq!(paths.len(), 1);
        assert!(paths.contains_key("baz/*"));
    }

    #[test]
    fn child_empty_paths_blanks_parent() {
        // A child `"paths": {}` blanks the parent's paths (present-but-
        // empty replaces, TS semantics).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("base.json"),
            r#"{ "compilerOptions": { "paths": { "foo/*": ["./foo/*"] } } }"#,
        )
        .unwrap();
        let ts = dir.path().join("tsconfig.json");
        std::fs::write(
            &ts,
            r#"{ "extends": "./base.json", "compilerOptions": { "paths": {} } }"#,
        )
        .unwrap();
        let cfg = load(&ts).unwrap();
        let paths = cfg.compiler_options.paths.as_ref().unwrap();
        assert!(
            paths.is_empty(),
            "child {{}} should blank parent: {paths:?}"
        );
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

    /// An extensionless package subpath picks up `.json`, the same
    /// append-don't-replace rule the relative branch uses. SvelteKit 3
    /// depends on it: `"extends": "$app/tsconfig"` names
    /// `node_modules/$app/tsconfig.json`, and a sibling `tsconfig/`
    /// directory (holding the service-worker config) sits next to it, so
    /// the literal path exists as a directory and must not win.
    #[test]
    fn package_extends_subpath_appends_json() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/$app");
        fs::create_dir_all(pkg_dir.join("tsconfig")).unwrap();
        write(
            &pkg_dir.join("tsconfig.json"),
            r#"{ "compilerOptions": { "rootDirs": ["../..", "../../.svelte-kit/types"] } }"#,
        );
        write(
            &pkg_dir.join("tsconfig/service-worker.json"),
            r#"{ "compilerOptions": { "target": "ES2015" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "$app/tsconfig" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(
            cfg.compiler_options.root_dirs,
            vec!["../..".to_string(), "../../.svelte-kit/types".to_string()]
        );
    }

    /// The append only fires when the literal path isn't already a file,
    /// so a reference that spells out `.json` still resolves.
    #[test]
    fn package_extends_subpath_with_json_suffix_resolves_literally() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/$app/tsconfig");
        fs::create_dir_all(&pkg_dir).unwrap();
        write(
            &pkg_dir.join("service-worker.json"),
            r#"{ "compilerOptions": { "target": "ES2015" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "$app/tsconfig/service-worker.json" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2015"));
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

    /// Conditions resolve in object key order, not a fixed priority
    /// list: `import` is inactive (skip), then `default` matches —
    /// before `require` is ever considered.
    #[test]
    fn package_exports_conditions_resolve_in_object_order() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": {
                "import": "./a.json",
                "default": "./b.json",
                "require": "./c.json"
            } } }"#,
        );
        write(
            &pkg_dir.join("a.json"),
            r#"{ "compilerOptions": { "target": "ES2015" } }"#,
        );
        write(
            &pkg_dir.join("b.json"),
            r#"{ "compilerOptions": { "target": "ES2020" } }"#,
        );
        write(
            &pkg_dir.join("c.json"),
            r#"{ "compilerOptions": { "target": "ES2022" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2020"));
    }

    /// Nested condition objects resolve recursively, again in object
    /// order: `node` matches, then inside it `import` is skipped and
    /// `require` wins.
    #[test]
    fn package_exports_nested_conditions_resolve_recursively() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": { "node": {
                "import": "./esm.json",
                "require": "./cjs.json"
            } } } }"#,
        );
        write(
            &pkg_dir.join("esm.json"),
            r#"{ "compilerOptions": { "target": "ES2015" } }"#,
        );
        write(
            &pkg_dir.join("cjs.json"),
            r#"{ "compilerOptions": { "target": "ES2022" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
    }

    /// Array targets take the first entry that resolves — here the
    /// condition object only offers the inactive `import`, so the
    /// string fallback wins.
    #[test]
    fn package_exports_array_target_takes_first_resolvable() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": [
                { "import": "./esm.json" },
                "./fallback.json"
            ] } }"#,
        );
        write(
            &pkg_dir.join("esm.json"),
            r#"{ "compilerOptions": { "target": "ES2015" } }"#,
        );
        write(
            &pkg_dir.join("fallback.json"),
            r#"{ "compilerOptions": { "target": "ES2022" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
    }

    /// A `*` pattern key captures the subpath remainder and substitutes
    /// it into the target.
    #[test]
    fn package_exports_wildcard_subpath_substitutes() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./*": "./configs/*.json" } }"#,
        );
        write(
            &pkg_dir.join("configs/base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/base" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    /// Among matching pattern keys the longest prefix before the `*`
    /// wins, per Node's PATTERN_KEY_COMPARE — regardless of the order
    /// the keys were written in.
    #[test]
    fn package_exports_longest_wildcard_prefix_wins() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": {
                "./*": "./loose/*.json",
                "./strict/*": "./strict/*.json"
            } }"#,
        );
        write(
            &pkg_dir.join("loose/strict/base.json"),
            r#"{ "compilerOptions": { "strict": false } }"#,
        );
        write(
            &pkg_dir.join("strict/base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/strict/base" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    /// Pattern targets resolve conditions the same way exact targets do,
    /// with the capture substituted inside the winning branch.
    #[test]
    fn package_exports_wildcard_with_conditions() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./*": {
                "import": "./esm/*.json",
                "default": "./any/*.json"
            } } }"#,
        );
        write(
            &pkg_dir.join("esm/base.json"),
            r#"{ "compilerOptions": { "target": "ES2015" } }"#,
        );
        write(
            &pkg_dir.join("any/base.json"),
            r#"{ "compilerOptions": { "target": "ES2022" } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/base" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.target.as_deref(), Some("ES2022"));
    }

    /// `exports` is exclusive: a subpath the map doesn't cover fails
    /// even though the file exists at the literal layout position.
    #[test]
    fn package_exports_uncovered_subpath_fails_despite_literal_file() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./other": "./other.json" } }"#,
        );
        write(
            &pkg_dir.join("other.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        // Exists on disk, but the exports map doesn't cover it.
        write(
            &pkg_dir.join("base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/base.json" }"#);

        let err = load(&ts).unwrap_err();
        assert!(
            matches!(err, LoadError::ExtendsNotFound { .. }),
            "expected ExtendsNotFound, got {err:?}"
        );
    }

    /// A package.json WITHOUT `exports` keeps the legacy file-layout
    /// probing (with the `.json` append).
    #[test]
    fn package_without_exports_still_probes_layout() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(&pkg_dir.join("package.json"), r#"{ "name": "cfg" }"#);
        write(
            &pkg_dir.join("base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/base" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    fn assert_extends_not_found(ts: &Path) {
        let err = load(ts).unwrap_err();
        assert!(
            matches!(err, LoadError::ExtendsNotFound { .. }),
            "expected ExtendsNotFound, got {err:?}"
        );
    }

    /// A condition whose target file does not exist fails that branch
    /// only — resolution falls through to the next condition, the way
    /// TS's per-branch load failure does.
    #[test]
    fn package_exports_condition_with_missing_target_falls_through() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": {
                "types": "./missing.json",
                "default": "./real.json"
            } } }"#,
        );
        write(
            &pkg_dir.join("real.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    /// Same fall-through for array targets: a missing first entry
    /// yields to the second.
    #[test]
    fn package_exports_array_with_missing_target_falls_through() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": ["./missing.json", "./real.json"] } }"#,
        );
        write(
            &pkg_dir.join("real.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        let cfg = load(&ts).unwrap();
        assert_eq!(cfg.compiler_options.strict, Some(true));
    }

    /// A target that escapes the package with `..` is invalid even
    /// though the escaped-to file exists.
    #[test]
    fn package_exports_rejects_target_escaping_package() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": { "types": "../real.json" } } }"#,
        );
        write(
            &tmp.path().join("node_modules/real.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);
        assert_extends_not_found(&ts);
    }

    /// A `node_modules` segment in the target is invalid, and a target
    /// not starting with `./` is invalid.
    #[test]
    fn package_exports_rejects_node_modules_segment_and_bare_targets() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("node_modules/inner.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        write(
            &pkg_dir.join("real.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": { "types": "./node_modules/inner.json" } } }"#,
        );
        assert_extends_not_found(&ts);

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": "real.json" } }"#,
        );
        assert_extends_not_found(&ts);
    }

    /// A pattern capture containing `..` is rejected — the subpath
    /// cannot smuggle a traversal through an otherwise-clean target.
    #[test]
    fn package_exports_rejects_dotdot_in_capture() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./cfg/*.json": "./configs/*.json" } }"#,
        );
        // Would resolve (configs/../real.json) if traversal were allowed.
        write(
            &pkg_dir.join("real.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/cfg/../real.json" }"#);
        assert_extends_not_found(&ts);
    }

    /// `types@<range>` condition keys match when the range admits the
    /// engine version (pinned to the 7.x family), and are skipped when
    /// it does not.
    #[test]
    fn package_exports_versioned_types_keys_test_engine_version() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("strict.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        write(
            &pkg_dir.join("loose.json"),
            r#"{ "compilerOptions": { "strict": false } }"#,
        );
        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": {
                "types@>=5.2": "./strict.json",
                "default": "./loose.json"
            } } }"#,
        );
        assert_eq!(load(&ts).unwrap().compiler_options.strict, Some(true));

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": {
                "types@<6": "./strict.json",
                "default": "./loose.json"
            } } }"#,
        );
        assert_eq!(load(&ts).unwrap().compiler_options.strict, Some(false));
    }

    /// Directory keys (trailing `/`) append the remainder to the
    /// target — which must itself end in `/` to be valid.
    #[test]
    fn package_exports_directory_key_resolves_and_requires_slash_target() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("configs/base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/cfg/base.json" }"#);

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./cfg/": "./configs/" } }"#,
        );
        assert_eq!(load(&ts).unwrap().compiler_options.strict, Some(true));

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./cfg/": "./configs" } }"#,
        );
        assert_extends_not_found(&ts);
    }

    /// The best-matching key is committed to: when its target fails to
    /// load, the lookup fails — no falling through to a lesser pattern
    /// or directory key that would have resolved. Same for exact keys.
    #[test]
    fn package_exports_matching_key_commits_despite_missing_target() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("configs/deep/base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        write(
            &pkg_dir.join("configs/base.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );

        // Pattern key with the longer base wins the sort, then fails.
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": {
                "./cfg/deep/*.json": "./missing/*.json",
                "./cfg/": "./configs/"
            } }"#,
        );
        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg/cfg/deep/base.json" }"#);
        assert_extends_not_found(&ts);

        // Exact key wins outright, then fails.
        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": {
                "./cfg/base.json": "./missing.json",
                "./cfg/": "./configs/"
            } }"#,
        );
        write(&ts, r#"{ "extends": "cfg/cfg/base.json" }"#);
        assert_extends_not_found(&ts);
    }

    /// `exports` is exclusive at the package root too: a `"."` that is
    /// absent or fails to resolve fails the package — no fallback to
    /// the `tsconfig` field or `tsconfig.json`, which serve only
    /// exports-less packages.
    #[test]
    fn package_exports_root_failure_does_not_fall_back_to_legacy() {
        let tmp = tempdir().unwrap();
        let pkg_dir = tmp.path().join("node_modules/cfg");
        write(
            &pkg_dir.join("tsconfig.json"),
            r#"{ "compilerOptions": { "strict": true } }"#,
        );
        let ts = tmp.path().join("tsconfig.json");
        write(&ts, r#"{ "extends": "cfg" }"#);

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { ".": "./missing.json" } }"#,
        );
        assert_extends_not_found(&ts);

        write(
            &pkg_dir.join("package.json"),
            r#"{ "exports": { "./other.json": "./tsconfig.json" } }"#,
        );
        assert_extends_not_found(&ts);
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
    fn load_chain_substitutes_config_dir_against_entry_dir() {
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
        // A base config's ${configDir} resolves against the ENTRY (child)
        // dir, not the base's own dir (TS semantics).
        let expected = child_dir.canonicalize().unwrap().join("src");
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
    fn winning_patterns_leaf_beats_parents() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "include": ["base/**/*"] }"#);
        write(
            &ts,
            r#"{ "extends": "./base.json", "include": ["src/**/*"] }"#,
        );

        let chain = load_chain(&ts).unwrap();
        let (winner, patterns) = winning_patterns(&chain, |f| f.include.as_deref()).unwrap();
        assert!(winner.path.ends_with("tsconfig.json"));
        assert_eq!(patterns, ["src/**/*"]);
    }

    #[test]
    fn winning_patterns_array_extends_last_entry_wins() {
        // extends: ["./a.json", "./b.json"] with the field declared only
        // in the parents — TS array-extends gives the LAST entry
        // precedence, exactly like merge_into's compilerOptions handling
        // (see load_with_array_extends_last_wins_for_conflicts).
        let tmp = tempdir().unwrap();
        let a = tmp.path().join("a.json");
        let b = tmp.path().join("b.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&a, r#"{ "include": ["app/**/*"] }"#);
        write(&b, r#"{ "include": ["src/**/*"] }"#);
        write(&ts, r#"{ "extends": ["./a.json", "./b.json"] }"#);

        let chain = load_chain(&ts).unwrap();
        let (winner, patterns) = winning_patterns(&chain, |f| f.include.as_deref()).unwrap();
        assert!(winner.path.ends_with("b.json"));
        assert_eq!(patterns, ["src/**/*"]);
    }

    #[test]
    fn winning_patterns_later_subtree_beats_earlier_declaration() {
        // extends: ["./a.json", "./b.json"]; a declares the field
        // directly, b inherits it from ITS parent. b's whole merged
        // subtree overrides a's, so b2's value wins even though it sits
        // deeper in the chain than a.
        let tmp = tempdir().unwrap();
        let a = tmp.path().join("a.json");
        let b = tmp.path().join("b.json");
        let b2 = tmp.path().join("b2.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&a, r#"{ "include": ["app/**/*"] }"#);
        write(&b, r#"{ "extends": "./b2.json" }"#);
        write(&b2, r#"{ "include": ["nested/**/*"] }"#);
        write(&ts, r#"{ "extends": ["./a.json", "./b.json"] }"#);

        let chain = load_chain(&ts).unwrap();
        let (winner, patterns) = winning_patterns(&chain, |f| f.include.as_deref()).unwrap();
        assert!(winner.path.ends_with("b2.json"));
        assert_eq!(patterns, ["nested/**/*"]);
    }

    #[test]
    fn winning_patterns_explicit_empty_array_is_a_declaration() {
        // `"include": []` on the leaf REPLACES a parent's include (TS
        // replace-on-child) — it must not be skipped as "not declared".
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "include": ["src/**/*"] }"#);
        write(&ts, r#"{ "extends": "./base.json", "include": [] }"#);

        let chain = load_chain(&ts).unwrap();
        let (winner, patterns) = winning_patterns(&chain, |f| f.include.as_deref()).unwrap();
        assert!(winner.path.ends_with("tsconfig.json"));
        assert!(patterns.is_empty());
    }

    #[test]
    fn winning_patterns_none_when_no_config_declares() {
        let tmp = tempdir().unwrap();
        let base = tmp.path().join("base.json");
        let ts = tmp.path().join("tsconfig.json");
        write(&base, r#"{ "compilerOptions": { "strict": true } }"#);
        write(&ts, r#"{ "extends": "./base.json" }"#);

        let chain = load_chain(&ts).unwrap();
        assert!(winning_patterns(&chain, |f| f.include.as_deref()).is_none());
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
