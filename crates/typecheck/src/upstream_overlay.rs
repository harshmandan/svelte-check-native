//! Where svelte-check reports a diagnostic the compiler attributes to the
//! overlay tsconfig.
//!
//! Both tools hand the compiler a generated tsconfig that `extends` the
//! project's, and a compiler-option or file-spec error lands on that
//! generated file. svelte-check writes it to `<workspace>/.svelte-check/
//! tsconfig.json` (`.svelte-kit/.svelte-check/` in a SvelteKit project)
//! and reports the error there, at the line its own layout puts the
//! offending entry on. Ours lives elsewhere and lays its entries out
//! differently, so such a diagnostic is re-anchored: the file becomes
//! svelte-check's overlay path, and the line becomes the line of the same
//! JSON entry in the overlay svelte-check would have written. Both files
//! are two-space pretty-printed JSON, so an entry at the same nesting
//! keeps its column.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use serde_json::{Map, Value, json};
use svn_core::tsconfig::TsConfigFile;

/// svelte-check's cache directory for `workspace`.
pub(crate) fn cache_dir(workspace: &Path) -> PathBuf {
    let kit = workspace.join(".svelte-kit");
    if kit.is_dir() {
        kit.join(".svelte-check")
    } else {
        workspace.join(".svelte-check")
    }
}

/// What svelte-check's overlay tsconfig is built from.
pub(crate) struct Inputs<'a> {
    pub workspace: &'a Path,
    pub user_tsconfig: &'a Path,
    /// The project's extends chain, entry config first.
    pub chain: &'a [TsConfigFile],
    /// Every source file that received an overlay: components, then kit
    /// files. svelte-check excludes each of them by path.
    pub emitted_sources: &'a [PathBuf],
}

/// The overlay tsconfig svelte-check writes for `inputs`
/// (`writeOverlayTsconfig`), with its entries in svelte-check's order.
pub(crate) fn overlay(inputs: &Inputs<'_>) -> Value {
    let overlay_dir = cache_dir(inputs.workspace);
    let tsconfig_dir = inputs.user_tsconfig.parent().unwrap_or(inputs.workspace);
    let rel = |target: &Path| relative_posix(&overlay_dir, target);
    let rebase = |spec: &str| -> String {
        let resolved = match spec.strip_prefix("${configDir}") {
            Some(rest) => tsconfig_dir.join(format!(".{rest}")),
            None if Path::new(spec).is_absolute() => PathBuf::from(spec),
            None => tsconfig_dir.join(spec),
        };
        rel(&normalize(&resolved))
    };
    let virtual_spec = |spec: &str| -> String {
        let spec = spec
            .strip_prefix("${configDir}")
            .map_or_else(|| spec.to_string(), |rest| format!(".{rest}"));
        let spec = spec
            .strip_suffix(".svelte")
            .map_or(spec.clone(), |stem| format!("{stem}.d.svelte.ts"));
        format!("svelte/{spec}")
    };
    let entry = inputs.chain.first();

    let mut root_dirs: Vec<String> = Vec::new();
    match svn_core::tsconfig::winning_field(inputs.chain, |f| {
        (!f.compiler_options.root_dirs.is_empty()).then_some(&f.compiler_options.root_dirs)
    }) {
        Some((file, dirs)) => {
            for dir in dirs {
                push_unique(
                    &mut root_dirs,
                    rel(&normalize(&file.config_dir().join(dir))),
                );
            }
        }
        None => push_unique(&mut root_dirs, rel(tsconfig_dir)),
    }
    push_unique(&mut root_dirs, rel(&overlay_dir.join("svelte")));

    let mut compiler_options = Map::new();
    compiler_options.insert("rootDirs".into(), json!(root_dirs));
    compiler_options.insert("allowArbitraryExtensions".into(), json!(true));
    compiler_options.insert("noEmit".into(), json!(true));
    compiler_options.insert("incremental".into(), json!(false));
    compiler_options.insert("tsBuildInfoFile".into(), json!("tsbuildinfo.json"));
    if let Some((file, paths)) =
        svn_core::tsconfig::winning_field(inputs.chain, |f| f.compiler_options.paths.as_ref())
        && !paths.is_empty()
    {
        let mut rebased = Map::new();
        for (pattern, specs) in paths {
            let mut values: Vec<String> = Vec::new();
            for spec in specs {
                let absolute = normalize(&file.config_dir().join(spec));
                values.push(rel(&absolute));
                let from_tsconfig = relative_posix(tsconfig_dir, &absolute);
                if !from_tsconfig.starts_with("../") {
                    values.push(format!("./svelte/{from_tsconfig}"));
                }
            }
            rebased.insert(pattern.clone(), json!(values));
        }
        compiler_options.insert("paths".into(), Value::Object(rebased));
    }

    let raw_files: Vec<String> = entry.and_then(|e| e.files.clone()).unwrap_or_default();
    let mut files: Vec<String> = Vec::new();
    for spec in raw_files.iter().filter(|s| !s.ends_with(".svelte")) {
        push_unique(&mut files, rebase(spec));
    }
    for spec in &raw_files {
        push_unique(&mut files, virtual_spec(spec));
    }
    // svelte2tsx's two shim declarations.
    push_unique(&mut files, "svelte-shims-v4.d.ts".into());
    push_unique(&mut files, "svelte-jsx-v4.d.ts".into());

    let raw_include = entry.and_then(|e| e.include.clone());
    let raw_exclude = entry.and_then(|e| e.exclude.clone());
    let mut include: Vec<String> = Vec::new();
    if let Some(specs) = &raw_include {
        include.extend(specs.iter().map(|s| rebase(s)));
        include.extend(specs.iter().map(|s| virtual_spec(s)));
    }
    let mut exclude: Vec<String> = Vec::new();
    if let Some(specs) = &raw_exclude {
        for spec in specs {
            push_unique(&mut exclude, rebase(spec));
        }
        for spec in specs {
            push_unique(&mut exclude, virtual_spec(spec));
        }
    }
    let mut seen: HashSet<String> = exclude.iter().cloned().collect();
    for source in inputs.emitted_sources {
        let value = rel(source);
        if seen.insert(value.clone()) {
            exclude.push(value);
        }
    }

    let mut out = Map::new();
    out.insert("extends".into(), json!(rel(inputs.user_tsconfig)));
    out.insert("compilerOptions".into(), Value::Object(compiler_options));
    out.insert("files".into(), json!(files));
    if !include.is_empty() {
        out.insert("include".into(), json!(include));
    }
    if !exclude.is_empty() {
        out.insert("exclude".into(), json!(exclude));
    }
    if let Some(entry) = entry.filter(|e| !e.references.is_empty()) {
        let references: Vec<Value> = entry
            .references
            .iter()
            .map(|r| json!({ "path": rebase(&r.path) }))
            .collect();
        out.insert("references".into(), Value::Array(references));
    }
    Value::Object(out)
}

fn push_unique(list: &mut Vec<String>, value: String) {
    if !list.contains(&value) {
        list.push(value);
    }
}

/// One segment of the JSON path a pretty-printed line belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Segment {
    Key(String),
    Index(usize),
}

/// The entry each line of a pretty-printed JSON value belongs to: the
/// path of the key or element that starts on it, or of the container a
/// closing bracket ends (flagged).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct LineEntry {
    path: Vec<Segment>,
    closes: bool,
}

/// Line entries of `value` as `serde_json::to_string_pretty` (and
/// `JSON.stringify(value, null, 2)`) lays it out: one line per key or
/// element, a non-empty container's closing bracket on a line of its own.
pub(crate) fn line_entries(value: &Value) -> Vec<LineEntry> {
    let mut out = vec![LineEntry {
        path: Vec::new(),
        closes: false,
    }];
    let mut path = Vec::new();
    push_container_lines(value, &mut path, &mut out);
    out
}

fn push_container_lines(value: &Value, path: &mut Vec<Segment>, out: &mut Vec<LineEntry>) {
    let children: Vec<(Segment, &Value)> = match value {
        Value::Object(map) if !map.is_empty() => map
            .iter()
            .map(|(k, v)| (Segment::Key(k.clone()), v))
            .collect(),
        Value::Array(items) if !items.is_empty() => items
            .iter()
            .enumerate()
            .map(|(i, v)| (Segment::Index(i), v))
            .collect(),
        _ => return,
    };
    for (segment, child) in children {
        path.push(segment);
        out.push(LineEntry {
            path: path.clone(),
            closes: false,
        });
        push_container_lines(child, path, out);
        path.pop();
    }
    out.push(LineEntry {
        path: path.clone(),
        closes: true,
    });
}

/// The 1-based line in `theirs` holding the entry that 1-based `line` of
/// `ours` holds, when `theirs` has that entry.
pub(crate) fn translate_line(ours: &[LineEntry], theirs: &[LineEntry], line: u32) -> Option<u32> {
    let wanted = ours.get(line.checked_sub(1)? as usize)?;
    theirs
        .iter()
        .position(|entry| entry == wanted)
        .map(|i| i as u32 + 1)
}

/// Node's `path.relative` in POSIX form, `.` for the same directory.
fn relative_posix(base: &Path, target: &Path) -> String {
    let base: Vec<Component<'_>> = base.components().collect();
    let target: Vec<Component<'_>> = target.components().collect();
    let common = base.iter().zip(&target).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = Vec::new();
    parts.extend(std::iter::repeat_n("..".to_string(), base.len() - common));
    parts.extend(
        target[common..]
            .iter()
            .map(|c| c.as_os_str().to_string_lossy().into_owned()),
    );
    if parts.is_empty() {
        ".".to_string()
    } else {
        parts.join("/")
    }
}

fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_paths_match_node() {
        let base = Path::new("/ws/.svelte-check");
        assert_eq!(
            relative_posix(base, Path::new("/ws/tsconfig.json")),
            "../tsconfig.json"
        );
        assert_eq!(
            relative_posix(base, Path::new("/ws/.svelte-check/svelte")),
            "svelte"
        );
        assert_eq!(relative_posix(base, Path::new("/ws/.svelte-check")), ".");
        assert_eq!(relative_posix(base, Path::new("/ws")), "..");
    }

    #[test]
    fn line_entries_follow_pretty_printing() {
        let value = json!({ "a": 1, "b": [1, 2], "c": {}, "d": { "e": [] } });
        let text = serde_json::to_string_pretty(&value).expect("serialises");
        let entries = line_entries(&value);
        assert_eq!(entries.len(), text.lines().count());
        let ours = line_entries(&json!({ "b": [1, 2], "x": 0 }));
        // `b[1]` is line 4 in ours and line 5 in `value`.
        assert_eq!(translate_line(&ours, &entries, 4), Some(5));
        // `x` has no counterpart.
        assert_eq!(translate_line(&ours, &entries, 6), None);
    }
}
