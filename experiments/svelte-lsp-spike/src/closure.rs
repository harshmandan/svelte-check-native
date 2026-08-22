//! Which `.svelte` files the open one actually needs.
//!
//! Only `.svelte` imports are followed. Everything else — `.ts`, `.js`,
//! packages — tsgo resolves for itself; we just have to make sure every
//! component in the graph has an overlay written before tsgo looks for one.
//!
//! The scan is textual rather than AST-based. That is a deliberate spike
//! shortcut: an over-broad closure costs a little memory and a correct
//! program, while a missing edge costs a bogus "cannot find module". Erring
//! toward including too much is the safe direction here.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Hard stop on the union scope. Past this, an editor session has told us the
/// file-level graph is not the right unit for the whole window, and the log
/// should say so rather than silently building a workspace-sized program.
pub const MAX_SCOPE: usize = 600;

/// Hard stop on closure size. A component that reaches this many others is
/// telling us the file-level graph is not the right unit, and we want to see
/// that in the log rather than silently build a workspace-sized program.
const MAX_FILES: usize = 400;

/// Breadth-first walk from `entry` over `.svelte` imports, returning the entry
/// plus everything reachable from it.
pub fn compute(entry: &Path, workspace: &Path) -> Vec<PathBuf> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut queue: Vec<PathBuf> = vec![entry.to_path_buf()];
    let mut out: Vec<PathBuf> = Vec::new();

    while let Some(file) = queue.pop() {
        if out.len() >= MAX_FILES {
            break;
        }
        let Ok(canonical) = file.canonicalize() else { continue };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(&canonical) else { continue };
        out.push(canonical.clone());

        for spec in specifiers(&source) {
            if let Some(resolved) = resolve(&spec, &canonical, workspace) {
                queue.push(resolved);
            }
        }
    }
    out
}

/// Every quoted module specifier in the file, from `import`/`export ... from`
/// and bare `import '...'`. Deliberately loose: a specifier that turns out not
/// to be a `.svelte` file is dropped by `resolve`.
fn specifiers(source: &str) -> Vec<String> {
    let bytes = source.as_bytes();
    let mut out = Vec::new();
    let mut i = 0usize;

    while let Some(found) = source[i..].find("from") {
        let start = i + found;
        i = start + 4;
        // Require a word boundary before `from` so `dateFrom` doesn't match.
        if start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_') {
            continue;
        }
        if let Some(spec) = quoted_after(source, i) {
            out.push(spec);
        }
    }

    // Side-effect imports: `import './Foo.svelte'`.
    let mut j = 0usize;
    while let Some(found) = source[j..].find("import") {
        let start = j + found;
        j = start + 6;
        if let Some(spec) = quoted_after(source, j) {
            out.push(spec);
        }
    }
    out
}

/// The next single- or double-quoted string starting at `from`, provided only
/// whitespace and an optional `(` separate them.
fn quoted_after(source: &str, from: usize) -> Option<String> {
    let rest = &source[from..];
    let mut chars = rest.char_indices();
    for (offset, ch) in &mut chars {
        match ch {
            ' ' | '\t' | '\n' | '\r' | '(' => continue,
            '"' | '\'' => {
                let body_start = from + offset + 1;
                let end = source[body_start..].find(ch)?;
                return Some(source[body_start..body_start + end].to_string());
            }
            _ => return None,
        }
    }
    None
}

/// Resolve a specifier to a `.svelte` file on disk, or `None` if it isn't one.
fn resolve(spec: &str, importer: &Path, workspace: &Path) -> Option<PathBuf> {
    if !spec.ends_with(".svelte") {
        return None;
    }
    let candidate = if let Some(rest) = spec.strip_prefix("$lib/") {
        workspace.join("src").join("lib").join(rest)
    } else if spec.starts_with("./") || spec.starts_with("../") {
        importer.parent()?.join(spec)
    } else {
        // Bare or aliased specifier. Aliases are a tsconfig `paths` question,
        // which this spike does not resolve — tsgo will, and a component we
        // miss shows up as a missing overlay rather than a wrong type.
        return None;
    };
    candidate.is_file().then_some(candidate)
}

/// The union of several entry files' closures, in most-recently-used order so
/// the cap drops the tab you touched longest ago rather than an arbitrary one.
///
/// A union rather than a scope per tab: the dependency declarations are ~88% of
/// any program here and paying for them once is the whole reason this design
/// fits in memory. Ten unrelated files measured 478 MB together against roughly
/// 1-2 GB as ten separate programs.
pub fn union(entries: &[PathBuf], workspace: &Path) -> Vec<PathBuf> {
    let mut seen: HashSet<PathBuf> = HashSet::new();
    let mut out: Vec<PathBuf> = Vec::new();
    for entry in entries {
        for file in compute(entry, workspace) {
            if out.len() >= MAX_SCOPE {
                return out;
            }
            if seen.insert(file.clone()) {
                out.push(file);
            }
        }
    }
    out
}
