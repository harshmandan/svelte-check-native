//! Path math: lexical normalisation, relative-path computation, and
//! the cross-workspace `../`-import rewriter that runs over each
//! overlay before it's written to the cache.
//!
//! Mirrors upstream svelte2tsx's
//! `helpers/rewriteExternalImports.ts` for the rewrite. The other
//! helpers are pure-Rust path utilities.

use std::path::{Path, PathBuf};

/// Rewrite `../`-starting import specifiers in overlay text so they
/// resolve correctly from the overlay's location (under
/// `<workspace>/node_modules/.cache/svelte-check-native/svelte/...`)
/// instead of from the source's location.
///
/// Mirrors upstream svelte2tsx's
/// `helpers/rewriteExternalImports.ts::getExternalImportRewrite`:
/// scan each import specifier; if it starts with `../` AND the resolved
/// target sits OUTSIDE `workspace`, rewrite the specifier to be relative
/// to the overlay's directory.
///
/// In-workspace `../`-imports stay as-is — they pass through TS's
/// `rootDirs` virtual mapping (the overlay tsconfig lists both the
/// source and cache directories as rootDirs).
///
/// Implementation: pure regex-style scan for `from "..."` /
/// `from '...'` / `import "..."` / `import('...')` patterns.
/// Conservative — if we misclassify a non-import string we'd just
/// change a string-literal value, which doesn't affect type-checking.
pub(crate) fn rewrite_external_imports(
    overlay_text: &str,
    source_path: &Path,
    overlay_path: &Path,
    workspace: &Path,
) -> String {
    let Some(source_dir) = source_path.parent() else {
        return overlay_text.to_string();
    };
    let Some(overlay_dir) = overlay_path.parent() else {
        return overlay_text.to_string();
    };

    let bytes = overlay_text.as_bytes();
    let mut out = String::with_capacity(overlay_text.len());
    let mut i = 0;
    let mut copy_from = 0;
    while i < bytes.len() {
        // Only ASCII quote bytes (`'` and `"`) are valid quote
        // delimiters in JS/TS string literals — multi-byte UTF-8
        // characters can't BE quote delimiters, so the byte-level
        // search is sound. The ASCII assumption only governs
        // quote-detection; the slice-copy below preserves all bytes
        // verbatim, multi-byte chars included.
        let quote = bytes[i];
        if (quote == b'\'' || quote == b'"')
            && bytes.get(i + 1) == Some(&b'.')
            && bytes.get(i + 2) == Some(&b'.')
            && bytes.get(i + 3) == Some(&b'/')
            && is_in_import_context(bytes, i)
        {
            // Find the matching closing quote (no escapes inside import
            // specifiers — JS/TS forbids them in module specifier strings).
            let spec_start = i + 1;
            let mut j = spec_start;
            while j < bytes.len() && bytes[j] != quote {
                j += 1;
            }
            if j >= bytes.len() {
                i += 1;
                continue;
            }
            let specifier = &overlay_text[spec_start..j];
            if let Some(rewritten) = compute_rewrite(specifier, source_dir, overlay_dir, workspace)
            {
                // Copy verbatim from `copy_from` up to (and including)
                // the opening quote.
                out.push_str(&overlay_text[copy_from..spec_start]);
                out.push_str(&rewritten);
                copy_from = j;
                i = j;
                continue;
            }
        }
        i += 1;
    }
    // Final tail.
    out.push_str(&overlay_text[copy_from..]);
    out
}

/// Check whether the byte position `quote_pos` is inside an
/// import-style context — preceded by `from `, `import(`, or
/// `import ` (with optional whitespace). Avoids rewriting plain
/// string literals like `const x = '../foo';` that aren't imports.
fn is_in_import_context(bytes: &[u8], quote_pos: usize) -> bool {
    // Walk backwards past whitespace.
    let mut i = quote_pos;
    while i > 0 && bytes[i - 1].is_ascii_whitespace() {
        i -= 1;
    }
    // Match `from`, `import`, or `import(` (with the `(` already past).
    let preceded_by = |needle: &[u8]| -> bool {
        i >= needle.len()
            && &bytes[i - needle.len()..i] == needle
            && (i == needle.len()
                || !bytes[i - needle.len() - 1].is_ascii_alphanumeric()
                    && bytes[i - needle.len() - 1] != b'_'
                    && bytes[i - needle.len() - 1] != b'$')
    };
    if preceded_by(b"from") {
        return true;
    }
    if preceded_by(b"import") {
        return true;
    }
    // `import("...")` — bytes immediately before quote position is `(`,
    // possibly with whitespace; `import` precedes the `(`.
    if i > 0 && bytes[i - 1] == b'(' {
        let mut k = i - 1;
        while k > 0 && bytes[k - 1].is_ascii_whitespace() {
            k -= 1;
        }
        if k >= b"import".len()
            && &bytes[k - b"import".len()..k] == b"import"
            && (k == b"import".len()
                || !bytes[k - b"import".len() - 1].is_ascii_alphanumeric()
                    && bytes[k - b"import".len() - 1] != b'_'
                    && bytes[k - b"import".len() - 1] != b'$')
        {
            return true;
        }
    }
    false
}

/// Compute the rewritten specifier, or `None` if no rewrite is
/// needed.
fn compute_rewrite(
    specifier: &str,
    source_dir: &Path,
    overlay_dir: &Path,
    workspace: &Path,
) -> Option<String> {
    if !specifier.starts_with("../") {
        return None;
    }
    let target = lexical_normalise(&source_dir.join(specifier));
    if is_within(&target, workspace) {
        return None;
    }
    let rewritten_path = path_relative(overlay_dir, &target)?;
    let rewritten = rewritten_path.to_string_lossy().replace('\\', "/");
    if rewritten == specifier {
        return None;
    }
    Some(rewritten)
}

/// Compute a relative path from `from_dir` to `to_path`, mirroring
/// Node's `path.relative` semantics for our two-absolute-path inputs.
pub(crate) fn path_relative(from_dir: &Path, to_path: &Path) -> Option<PathBuf> {
    let from = lexical_normalise(from_dir);
    let to = lexical_normalise(to_path);
    let from_components: Vec<_> = from.components().collect();
    let to_components: Vec<_> = to.components().collect();
    let common_len = from_components
        .iter()
        .zip(to_components.iter())
        .take_while(|(a, b)| a == b)
        .count();
    let mut out = PathBuf::new();
    for _ in 0..(from_components.len() - common_len) {
        out.push("..");
    }
    for component in &to_components[common_len..] {
        out.push(component);
    }
    Some(out)
}

/// Is `target` inside `dir` lexically?
pub(crate) fn is_within(target: &Path, dir: &Path) -> bool {
    let target_n = lexical_normalise(target);
    let dir_n = lexical_normalise(dir);
    target_n.starts_with(&dir_n)
}

/// Resolve `.` and `..` components of `p` lexically — without touching
/// the filesystem. Used to normalise tsgo's relative-with-`..` paths
/// after they've been joined onto a workspace root.
///
/// `dunce::canonicalize` would also resolve symlinks, but requires the
/// file to exist. Lexical normalisation works on virtual paths (the
/// cache may be written but tsgo's `..`-formed path may not literally
/// exist as that string). Mirrors the path-clean crate's algorithm.
pub(crate) fn lexical_normalise(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    let mut has_root = false;
    for component in p.components() {
        match component {
            Component::Prefix(_) | Component::RootDir => {
                has_root = true;
                out.push(component.as_os_str());
            }
            Component::CurDir => {}
            Component::ParentDir => {
                let last = out.components().next_back();
                match last {
                    Some(Component::Normal(_)) => {
                        out.pop();
                    }
                    Some(Component::ParentDir) | None => {
                        // Leading `..` chain on a relative path is
                        // preserved — there's nothing to pop against.
                        out.push(component.as_os_str());
                    }
                    _ if has_root => {
                        // `..` past the root collapses to the root
                        // (Unix `cd /..` stays at `/`).
                    }
                    _ => out.push(component.as_os_str()),
                }
            }
            Component::Normal(_) => out.push(component.as_os_str()),
        }
    }
    out
}
