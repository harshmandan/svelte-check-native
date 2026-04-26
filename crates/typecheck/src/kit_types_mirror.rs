//! Synthetic `$types.d.ts` mirror — closes the SvelteKit
//! `$types.d.ts → user-source-+page.ts` transitive-resolution leak.
//!
//! Background. svelte-kit `sync` writes per-route `$types.d.ts` files
//! under `<workspace>/.svelte-kit/types/src/routes/<route>/$types.d.ts`.
//! Each file's `PageData` references the user's load function via a
//! relative import chain, e.g.
//!
//! ```ts
//! export type PageData = … typeof import('../../../../../../../src/routes/<route>/+page.js').load …;
//! ```
//!
//! The `..` chain is hand-counted by svelte-kit to walk back from
//! `.svelte-kit/types/src/routes/<route>/` to `<workspace>/src/routes/<route>/+page.js`
//! — the USER's source, which is untyped. tsgo loads that file
//! independently of our overlay and reports implicit-any on its
//! parameters; the cascade widens `data: PageData` to `any` at every
//! consumer `.svelte` site.
//!
//! Fix: write a copy of every `$types.d.ts` into the cache at
//! [`CacheLayout::kit_types_mirror_dir`] with each
//! `../(…/)src/routes/` substring rewritten to `../(…/)svelte/src/routes/`,
//! so the chain lands inside our typed Kit-file copies under
//! [`CacheLayout::svelte_dir`] instead. The cache mirror dir wins
//! against the user's `.svelte-kit/types/` via the overlay tsconfig's
//! `rootDirs` priority (cache mirror listed first).
//!
//! Critical companion: the overlay's inherited `include` glob that
//! targets `**/.svelte-kit/types/**/$types.d.ts` MUST be rewritten to
//! the cache mirror — without that, the user `$types.d.ts` files
//! stay in the file set and the leak persists. See
//! [`crate::overlay::build`].
//!
//! No-op when `<workspace>/.svelte-kit/types/` doesn't exist (the
//! user hasn't run `svelte-kit sync` yet, or the project isn't a
//! SvelteKit project at all).

use std::path::PathBuf;

use svn_core::sveltekit::{KitFilesSettings, user_source_needles};

use crate::cache::{CacheLayout, write_if_changed};

/// Walk the user's `.svelte-kit/types/` tree, write a path-rewritten
/// copy of every `$types.d.ts` into the cache mirror, and GC any
/// previously-mirrored files whose source has been deleted or
/// renamed.
///
/// Returns the mirror dir if at least one file was written (so the
/// overlay builder knows to enable the rootDirs priority + include-
/// glob rewrite), or `None` if there's no user `.svelte-kit/types/`
/// to mirror.
pub fn sync_mirror(layout: &CacheLayout) -> std::io::Result<Option<PathBuf>> {
    let user_types_root = layout.workspace.join(".svelte-kit").join("types");
    if !user_types_root.is_dir() {
        return Ok(None);
    }
    let mirror_root = layout.kit_types_mirror_dir();
    // Pull the user-source needle list from the centralised primitive
    // so the rewriter stays in lockstep with discovery's classifier.
    // Defaults are used here because `sync_mirror`'s call chain
    // (`typecheck::check`) doesn't currently thread the parsed
    // svelte.config.js settings down — fine today since today's
    // `user_source_needles` doesn't read any settings field. When
    // hooks/params get added (and the cache copies catch up), the
    // settings need to be plumbed through this call site.
    let settings = KitFilesSettings::default();
    let needles = user_source_needles(&settings);
    let mut wrote_any = false;
    let mut written: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();
    for entry in walkdir::WalkDir::new(&user_types_root)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        if !entry.file_type().is_file() {
            continue;
        }
        let path = entry.path();
        // Mirror $types.d.ts files (the leaky ones) plus any sibling
        // declaration files svelte-kit emits — they reference each
        // other and the same path-rewrite is safe.
        if !path
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with(".d.ts"))
        {
            continue;
        }
        let rel = match path.strip_prefix(&user_types_root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let content = std::fs::read_to_string(path)?;
        let rewritten = rewrite_user_source_chain(&content, &needles);
        let out = mirror_root.join(rel);
        write_if_changed(&out, &rewritten)?;
        written.insert(out);
        wrote_any = true;
    }
    // GC orphans. A deleted/renamed route leaves its `$types.d.ts`
    // in the cache mirror forever otherwise; tsgo's overlay program
    // then keeps consulting the stale typing instead of firing
    // 'cannot find module' / picking up the user's intended renames.
    // Best-effort: errors during traversal or deletion don't fail
    // the type-check (a stale orphan is recoverable next run).
    if mirror_root.is_dir() {
        for entry in walkdir::WalkDir::new(&mirror_root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().is_file())
        {
            let path = entry.path();
            if !written.contains(path) {
                let _ = std::fs::remove_file(path);
            }
        }
    }
    Ok(if wrote_any { Some(mirror_root) } else { None })
}

/// Rewrite every `'../(…/)src/routes/'` substring inside the file to
/// `'../(…/)svelte/src/routes/'`. The leading `'../'` chain length
/// is preserved — only the segment NAME is shifted: `src/routes/`
/// becomes `svelte/src/routes/`, redirecting the chain into the
/// cache's typed Kit-file copies.
///
/// Hooks (`src/hooks.{server,client,…}.{js,ts}`) and param matchers
/// (`src/params/<matcher>.{js,ts}`) are deliberately NOT rewritten:
/// `kit_inject` doesn't materialise cache copies for those (it only
/// classifies `+server` / `+page` / `+layout` shapes today), so a
/// rewritten chain points at a non-existent `<cache>/svelte/src/hooks…`.
/// tsgo's rootDirs fallback would still reach the user-tree source
/// via `<workspace>/src/hooks…`, but only by accident — leaving the
/// chain as-is keeps resolution direct and avoids the rootDirs round-
/// trip entirely. When `kit_inject` learns to type hooks/params (the
/// `Handle` / `HandleFetch` / `HandleServerError` / `ParamMatcher`
/// surfaces upstream svelte2tsx covers), the cache copy will appear
/// and we can extend the rewriter at the same time.
///
/// Conservative substring match: only rewrites occurrences preceded
/// by `../` (i.e. inside an existing relative-walk chain). A literal
/// `src/routes/` in a comment won't false-match because it lacks the
/// leading `../` that the SvelteKit-generated chains always have.
///
/// `needles` comes from `svn_core::sveltekit::user_source_needles` so
/// the rewriter's recognition list tracks the centralised classifier
/// rather than being hardcoded.
fn rewrite_user_source_chain(text: &str, needles: &[String]) -> String {
    let mut out = String::with_capacity(text.len() + 32);
    let mut rest = text;
    while let Some(idx) = find_user_source_segment(rest, needles) {
        out.push_str(&rest[..idx]);
        out.push_str("svelte/");
        rest = &rest[idx..];
    }
    out.push_str(rest);
    out
}

/// Find the byte offset of the earliest user-source substring in
/// `text` preceded by `../` (the SvelteKit chain signature). Returns
/// the START of that segment; the caller inserts `svelte/` at that
/// position.
fn find_user_source_segment(text: &str, needles: &[String]) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut best: Option<usize> = None;
    for needle in needles {
        let nb = needle.as_bytes();
        let mut i = 0usize;
        while let Some(rel) = bytes[i..].windows(nb.len()).position(|w| w == nb) {
            let pos = i + rel;
            if pos >= 3 && &bytes[pos - 3..pos] == b"../" {
                best = match best {
                    Some(prev) if prev <= pos => Some(prev),
                    _ => Some(pos),
                };
                break; // earliest hit FOR THIS NEEDLE; later ones can't beat it
            }
            i = pos + nb.len();
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default needle list — what callers get from
    /// `user_source_needles(&KitFilesSettings::default())` today.
    fn needles() -> Vec<String> {
        user_source_needles(&KitFilesSettings::default())
    }

    #[test]
    fn rewrites_simple_routes_chain() {
        let input = "typeof import('../../../../../../../src/routes/foo/+page.js').load";
        let want = "typeof import('../../../../../../../svelte/src/routes/foo/+page.js').load";
        assert_eq!(rewrite_user_source_chain(input, &needles()), want);
    }

    #[test]
    fn rewrites_layout_parent_data_chain() {
        let input = "type PageParentData = EnsureDefined<import('../../../../../src/routes/+page.js').load>;";
        let want = "type PageParentData = EnsureDefined<import('../../../../../svelte/src/routes/+page.js').load>;";
        assert_eq!(rewrite_user_source_chain(input, &needles()), want);
    }

    #[test]
    fn leaves_relative_dollar_types_imports_alone() {
        // `import('../$types.js')` walks within the mirror itself —
        // must not be rewritten, otherwise it'd land in svelte/$types
        // which doesn't exist.
        let input = "type X = import('../$types.js').LayoutData;";
        assert_eq!(rewrite_user_source_chain(input, &needles()), input);
    }

    #[test]
    fn rewrites_multiple_chains_in_one_file() {
        let input =
            "import('../../../src/routes/a/+page.js'); import('../../../src/routes/b/+page.js');";
        let want = "import('../../../svelte/src/routes/a/+page.js'); import('../../../svelte/src/routes/b/+page.js');";
        assert_eq!(rewrite_user_source_chain(input, &needles()), want);
    }

    #[test]
    fn leaves_hooks_chain_alone() {
        // Hooks are intentionally NOT rewritten — `kit_inject` doesn't
        // materialise a `<cache>/svelte/src/hooks.*` copy, so redirecting
        // the chain there would dangle. See `rewrite_user_source_chain`
        // doc comment for the future-extension note.
        let input = "typeof import('../../src/hooks.server.js').handle";
        assert_eq!(rewrite_user_source_chain(input, &needles()), input);
    }

    #[test]
    fn leaves_params_chain_alone() {
        let input = "import('../../src/params/videoId.js').match";
        assert_eq!(rewrite_user_source_chain(input, &needles()), input);
    }

    #[test]
    fn does_not_rewrite_bare_src_routes_without_dotdot() {
        // A literal occurrence not preceded by `../` is not a chain
        // segment we should touch.
        let input = "// in src/routes/ we keep things tidy";
        assert_eq!(rewrite_user_source_chain(input, &needles()), input);
    }

    #[test]
    fn idempotent_after_one_pass() {
        let input = "import('../../../src/routes/foo/+page.js');";
        let once = rewrite_user_source_chain(input, &needles());
        let twice = rewrite_user_source_chain(&once, &needles());
        assert_eq!(once, twice);
    }

    #[test]
    fn rewrites_routes_alongside_unmodified_hooks() {
        // Mixed input: hooks and params chains appear alongside a
        // routes chain in the same file. Only the routes chain is
        // redirected; hooks/params stay pointing at the user tree
        // because `kit_inject` doesn't produce cache copies for them.
        let input =
            "x: import('../../src/hooks.server.js'); y: import('../../src/routes/a/+page.js');";
        let want = "x: import('../../src/hooks.server.js'); y: import('../../svelte/src/routes/a/+page.js');";
        assert_eq!(rewrite_user_source_chain(input, &needles()), want);
    }

    #[test]
    fn parametric_custom_needles_are_honoured() {
        // Sanity check that the rewriter consumes the needles slice
        // (rather than a hardcoded list). When the centralised
        // `user_source_needles` grows hooks/params support, this same
        // slice plumbing carries the new needles through with no
        // additional rewriter changes.
        let input = "import('../../src/myroutes/foo/+page.js');";
        let want = "import('../../svelte/src/myroutes/foo/+page.js');";
        let custom = vec!["src/myroutes/".to_string()];
        assert_eq!(rewrite_user_source_chain(input, &custom), want);
    }
}

/// Pattern matcher used by [`crate::overlay::build`] to detect an
/// inherited `include` glob targeting `.svelte-kit/types/**/$types.d.ts`
/// or any of its near-equivalents svelte-kit's tsconfig has emitted
/// across versions. Returns the offset+length of the user-tree
/// `.svelte-kit/types` segment so the caller can swap it for the
/// cache mirror dir.
pub fn find_kit_types_segment(pattern: &str) -> Option<(usize, usize)> {
    const TARGETS: &[&str] = &[".svelte-kit/types"];
    for t in TARGETS {
        if let Some(pos) = pattern.find(t) {
            return Some((pos, t.len()));
        }
    }
    None
}

#[cfg(test)]
mod glob_rewrite_tests {
    use super::*;

    #[test]
    fn finds_kit_types_in_relative_glob() {
        let pat = "./.svelte-kit/types/**/$types.d.ts";
        let (start, len) = find_kit_types_segment(pat).unwrap();
        assert_eq!(&pat[start..start + len], ".svelte-kit/types");
    }

    #[test]
    fn finds_kit_types_in_absolute_glob() {
        let pat = "/abs/path/to/workspace/.svelte-kit/types/**/$types.d.ts";
        let (start, len) = find_kit_types_segment(pat).unwrap();
        assert_eq!(&pat[start..start + len], ".svelte-kit/types");
    }

    #[test]
    fn returns_none_for_unrelated_glob() {
        assert!(find_kit_types_segment("src/**/*.ts").is_none());
        assert!(find_kit_types_segment("**/*.svelte").is_none());
    }
}
