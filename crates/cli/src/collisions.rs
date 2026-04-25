//! `.svelte` import-specifier rewriting for the `Foo.svelte` /
//! `Foo.svelte.ts` sibling-collision case.
//!
//! When a user `.ts` file imports `./Foo.svelte` and a `Foo.svelte.ts`
//! runes module sits beside it, tsgo's `rootDirs` resolver picks the
//! user's source tree (longest matching prefix), then auto-extends
//! `.svelte` to `.svelte.ts` and lands on the runes module — which has
//! named exports but no `default`, firing TS2305. Rewriting the import
//! specifier to `.svelte.svn.js` in an overlay sidesteps the
//! auto-extension entirely; tsgo resolves via bundler module
//! resolution straight to the cache-side `.svelte.svn.ts`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Scan a user `.ts` file for `from '<relpath>.svelte'` /
/// `from "<relpath>.svelte"` specifiers and rewrite each to end in
/// `.svelte.svn.js` when (and only when) the target's sibling
/// `<relpath>.svelte.ts` file exists in `runes_modules`.
///
/// Returns `Some(rewritten)` if at least one specifier was rewritten,
/// `None` if the file contains no collision-case imports (caller then
/// skips creating an overlay — the file type-checks via the normal
/// `include` glob).
///
/// Scope is deliberately narrow: only relative specifiers that end
/// with the literal `.svelte` extension. Non-relative imports
/// (`@foo/bar.svelte`, aliased `$lib/x.svelte`) bypass this path —
/// those either resolve via `paths` aliases in the overlay tsconfig
/// or aren't subject to the rootDirs collision. Only the `from` form
/// of a static import/export is scanned; `import('./X.svelte')`
/// dynamic imports are rare enough in `.ts` files to defer.
pub(crate) fn rewrite_svelte_imports_for_collisions(
    file: &Path,
    source: &str,
    runes_modules: &HashSet<PathBuf>,
) -> Option<String> {
    let file_dir = file.parent()?;
    let bytes = source.as_bytes();
    let mut rewrites: Vec<(usize, usize, String)> = Vec::new();
    // Walk `from "..."` / `from '...'` patterns. Simple byte scan —
    // `from ` is followed by whitespace then an opening quote. The
    // TS grammar guarantees the specifier is a single string literal
    // (no template literals / concatenation allowed in a static
    // import).
    let mut i = 0;
    while i + 4 < bytes.len() {
        // Look for `from` preceded by whitespace or `}` (typical after
        // re-export list) and followed by whitespace + `'` or `"`.
        if &bytes[i..i + 4] == b"from"
            && (i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'}'))
        {
            let mut j = i + 4;
            while j < bytes.len() && matches!(bytes[j], b' ' | b'\t') {
                j += 1;
            }
            if j < bytes.len() && (bytes[j] == b'\'' || bytes[j] == b'"') {
                let quote = bytes[j];
                let spec_start = j + 1;
                let Some(offset) = bytes[spec_start..].iter().position(|&b| b == quote) else {
                    i = j + 1;
                    continue;
                };
                let spec_end = spec_start + offset;
                let spec = &source[spec_start..spec_end];
                if spec.ends_with(".svelte") && (spec.starts_with("./") || spec.starts_with("../"))
                {
                    let target = file_dir.join(spec);
                    // Collision requires BOTH siblings. A standalone
                    // `.svelte.ts` runes module with no matching
                    // `.svelte` component (Svelte 5 convention for
                    // pure-TS rune stores) means the user's
                    // `import './foo.svelte'` is intended to resolve to
                    // the runes module via bundler auto-extension —
                    // not a collision. Skip those.
                    if !target.is_file() {
                        i = spec_end + 1;
                        continue;
                    }
                    let sibling_runes = target.with_file_name(format!(
                        "{}.ts",
                        target.file_name().and_then(|s| s.to_str()).unwrap_or("")
                    ));
                    let sibling_canon = dunce::canonicalize(&sibling_runes)
                        .ok()
                        .unwrap_or(sibling_runes);
                    if runes_modules.contains(&sibling_canon) {
                        // Rewrite `<spec>` → `<spec>.svn.js`.
                        let replacement = format!("{spec}.svn.js");
                        rewrites.push((spec_start, spec_end, replacement));
                    }
                }
                i = spec_end + 1;
                continue;
            }
        }
        i += 1;
    }
    if rewrites.is_empty() {
        return None;
    }
    // Apply rewrites in reverse order so earlier byte offsets stay
    // valid while later splices happen first.
    let mut out = source.to_string();
    for (start, end, replacement) in rewrites.into_iter().rev() {
        out.replace_range(start..end, &replacement);
    }
    Some(out)
}
