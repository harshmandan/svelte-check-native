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

use std::path::{Path, PathBuf};

use rayon::prelude::*;
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
///
/// The per-file read+rewrite+compare body fans out over rayon — each
/// file's work is independent (distinct mirror target paths, no shared
/// mutable state), and large Kit route trees have a thousand-plus
/// generated `.d.ts` files, so a serial pass costs tens of
/// milliseconds of pure IO latency per run.
pub fn sync_mirror(layout: &CacheLayout) -> std::io::Result<Option<PathBuf>> {
    let user_types_root = layout.workspace.join(".svelte-kit").join("types");
    if !user_types_root.is_dir() {
        return Ok(None);
    }
    let mirror_root = layout.kit_types_mirror_dir();
    // Pull the user-source needle list from the centralised primitive
    // so the rewriter stays in lockstep with discovery's classifier.
    // Defaults are used here because `sync_mirror`'s call chain
    // (`typecheck::check`) doesn't thread the parsed config settings
    // down — fine as long as `user_source_needles` reads no settings
    // field. The day it does, this call site needs the real settings
    // plumbed to it.
    let settings = KitFilesSettings::default();
    let needles = user_source_needles(&settings);
    // Enumerate mirror candidates serially (directory traversal is
    // ordering-sensitive in walkdir and cheap relative to the file
    // IO), then fan the per-file work out.
    //
    // Mirror $types.d.ts files (the leaky ones) plus any sibling
    // declaration files svelte-kit emits — they reference each
    // other and the same path-rewrite is safe.
    let candidates: Vec<PathBuf> = walkdir::WalkDir::new(&user_types_root)
        .into_iter()
        .filter_map(|e| e.ok())
        .filter(|entry| {
            entry.file_type().is_file()
                && entry
                    .path()
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with(".d.ts"))
        })
        .map(|entry| entry.into_path())
        .collect();
    let written: std::collections::HashSet<PathBuf> = candidates
        .into_par_iter()
        .filter_map(|path| {
            let rel = match path.strip_prefix(&user_types_root) {
                Ok(r) => r,
                Err(_) => return None,
            };
            let out = mirror_root.join(rel);
            let original_dir = path.parent().unwrap_or(&user_types_root).to_path_buf();
            Some(std::fs::read_to_string(&path).and_then(|content| {
                let rewritten = rewrite_relative_chains(
                    &content,
                    &original_dir,
                    &user_types_root,
                    &mirror_root,
                    layout,
                    &needles,
                );
                write_if_changed(&out, &rewritten)?;
                Ok(out)
            }))
        })
        .collect::<std::io::Result<_>>()?;
    let wrote_any = !written.is_empty();
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

/// Re-anchor every relative module specifier in a generated
/// `$types.d.ts` so that copying the file into the cache cannot change
/// what it points at.
///
/// This is the invariant the mirror lives or dies by: **no relative
/// specifier may survive a directory move unrewritten.** The copy sits
/// at the same depth relative to the cache mirror root as the original
/// does to `.svelte-kit/types/`, but those two roots are at different
/// depths in the tree, so an untouched `../../../x` walks out to a
/// completely different place.
///
/// Three destinations:
///
/// - A specifier landing on a user-source path we generate typed
///   overlays for (the `needles`, e.g. `src/routes/`) is redirected to
///   that overlay under `<cache>/svelte/`. This is the mirror's whole
///   purpose — without it the chain walks back to the user's untyped
///   source and the typing we generated is ignored.
/// - A specifier landing back inside the generated tree we mirror from
///   (sibling/ancestor `$types.js` chains — a child route's
///   `import('../$types.js').LayoutData` reaching its parent) is
///   re-anchored onto the corresponding copy under the mirror root.
///   Pointing it at the un-mirrored original instead would re-open the
///   leak: the original's own chains are unrewritten, so they walk back
///   to the very un-typed user source this whole module exists to
///   shadow.
/// - Anything else is rewritten to the absolute path of the file the
///   compiler would have loaded had the `$types.d.ts` stayed put.
///
/// The last case is not hypothetical, and the previous rewriter's
/// claim that only route chains occur was wrong. SvelteKit names param
/// matchers by user-tree path: a `[foo=matcher]` route generates
/// `MatcherParam<typeof import('../../../src/params/foo.js').match>`.
/// Left dangling, that import resolved to nothing, the matched param
/// silently widened to `any`, and every misuse of it went unreported —
/// or, with `skipLibCheck` off, surfaced as a TS2307 against a
/// generated file the user cannot edit.
fn rewrite_relative_chains(
    text: &str,
    original_dir: &Path,
    source_root: &Path,
    mirror_root: &Path,
    layout: &CacheLayout,
    needles: &[String],
) -> String {
    let mut out = String::with_capacity(text.len() + 64);
    let bytes = text.as_bytes();
    let mut i = 0usize;
    while i < bytes.len() {
        // Copy everything up to the next quote verbatim, as a slice —
        // never byte-by-byte, which would decode multibyte UTF-8 as one
        // mangled char per byte. Quotes are ASCII, so the found offset
        // is always a char boundary.
        let Some(rel_quote) = text[i..].find(['\'', '"']) else {
            out.push_str(&text[i..]);
            break;
        };
        out.push_str(&text[i..i + rel_quote]);
        i += rel_quote;
        // A quoted run. Find its close on the same line; an unterminated
        // quote is not something a generated file produces, but bail
        // safely rather than scanning to EOF.
        let quote = bytes[i];
        let Some(rel_end) = text[i + 1..].find([quote as char, '\n']) else {
            out.push_str(&text[i..]);
            return out;
        };
        let end = i + 1 + rel_end;
        if bytes[end] != quote {
            out.push_str(&text[i..=end]);
            i = end + 1;
            continue;
        }
        let inner = &text[i + 1..end];
        match reanchor_specifier(
            inner,
            original_dir,
            source_root,
            mirror_root,
            layout,
            needles,
        ) {
            Some(replacement) => {
                out.push(quote as char);
                out.push_str(&replacement);
                out.push(quote as char);
            }
            None => out.push_str(&text[i..=end]),
        }
        i = end + 1;
    }
    out
}

/// Resolve one specifier against the original file's directory and
/// return its replacement, or `None` to leave it untouched (bare
/// package specifiers, non-paths, anything outside the workspace).
fn reanchor_specifier(
    spec: &str,
    original_dir: &Path,
    source_root: &Path,
    mirror_root: &Path,
    layout: &CacheLayout,
    needles: &[String],
) -> Option<String> {
    if !(spec.starts_with("./") || spec.starts_with("../")) {
        return None;
    }
    let resolved = lexically_normalise(&original_dir.join(spec));
    let rel = resolved.strip_prefix(&layout.workspace).ok()?;
    let rel_str = rel.to_string_lossy().replace('\\', "/");
    for needle in needles {
        if let Some(stripped) = rel_str.strip_prefix(needle.as_str()) {
            let mut target = layout.svelte_dir.join(needle.trim_end_matches('/'));
            target.push(stripped);
            return Some(target.to_string_lossy().replace('\\', "/"));
        }
    }
    // A chain between generated files (a child route's
    // `import('../$types.js')` reaching its parent's `$types.d.ts`)
    // must stay inside the mirror. The un-mirrored original at the
    // same spot carries unrewritten chains of its own, so anchoring
    // there would pull the raw user tree back into the program.
    if let Ok(inside) = resolved.strip_prefix(source_root) {
        return Some(
            mirror_root
                .join(inside)
                .to_string_lossy()
                .replace('\\', "/"),
        );
    }
    Some(resolved.to_string_lossy().replace('\\', "/"))
}

/// `..`/`.` collapsing without touching the filesystem — the targets
/// are generated paths that need not exist yet.
fn lexically_normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Default needle list — what callers get from
    /// `user_source_needles(&KitFilesSettings::default())` today.
    fn needles() -> Vec<String> {
        user_source_needles(&KitFilesSettings::default())
    }

    /// Stand-in layout. `/ws` is the workspace; the cache and its
    /// `svelte/` overlay dir hang off it as they do in a real run.
    fn layout() -> CacheLayout {
        CacheLayout::for_workspace("/ws")
    }

    /// A generated `$types.d.ts` lives this deep under the user's
    /// `.svelte-kit/types/`, so its chains walk up from here. Five
    /// segments below the workspace (`.svelte-kit`, `types`, `src`,
    /// `routes`, `foo`), which is why the fixtures below use five
    /// `../` to reach user source — the same shape SvelteKit emits.
    fn original_dir() -> PathBuf {
        PathBuf::from("/ws/.svelte-kit/types/src/routes/foo")
    }

    fn rewrite(input: &str) -> String {
        rewrite_from(input, &original_dir())
    }

    /// Like [`rewrite`] but with the original file placed elsewhere in
    /// the generated tree — for exercising sibling/ancestor chains
    /// between routes at different depths.
    fn rewrite_from(input: &str, original_dir: &Path) -> String {
        let layout = layout();
        let source_root = PathBuf::from("/ws/.svelte-kit/types");
        let mirror_root = layout.kit_types_mirror_dir();
        rewrite_relative_chains(
            input,
            original_dir,
            &source_root,
            &mirror_root,
            &layout,
            &needles(),
        )
    }

    #[test]
    fn route_chains_are_redirected_to_the_generated_overlay() {
        // `src/routes/` is a needle: the chain must land on our typed
        // copy under `<cache>/svelte/`, not on the user's source, or
        // the typing the mirror exists to install is ignored.
        let input = "typeof import('../../../../../src/routes/foo/+page.js').load";
        let got = rewrite(input);
        assert!(
            got.contains("/ws/.svelte-check/svelte/src/routes/foo/+page.js"),
            "route chain not redirected into the cache: {got}"
        );
    }

    #[test]
    fn param_matcher_chains_are_re_anchored_on_the_user_source() {
        // The case the old rewriter left dangling. `src/params/` is not
        // a needle — there is no generated overlay for it — so the
        // chain must point at the real file by absolute path. Left
        // relative, it walked out of the cache to nowhere and the
        // matched param silently widened to `any`.
        let input = "import('../../../../../src/params/videoId.js').match";
        let got = rewrite(input);
        assert!(
            got.contains("'/ws/src/params/videoId.js'"),
            "param matcher chain not re-anchored: {got}"
        );
    }

    #[test]
    fn hooks_chains_are_re_anchored_on_the_user_source() {
        let input = "typeof import('../../../../../src/hooks.server.js').handle";
        let got = rewrite(input);
        assert!(
            got.contains("'/ws/src/hooks.server.js'"),
            "hooks chain not re-anchored: {got}"
        );
    }

    #[test]
    fn sibling_dollar_types_imports_stay_inside_the_mirror() {
        // `import('../$types.js')` refers to another generated file —
        // a child route reaching its parent's `$types.d.ts`. It must
        // land on the parent's MIRRORED copy: the un-mirrored original
        // under `.svelte-kit/types/` carries unrewritten chains of its
        // own, so anchoring there would pull the raw user tree back
        // into the program.
        let input = "type X = import('../$types.js').LayoutData;";
        let got = rewrite(input);
        assert!(
            got.contains("'/ws/.svelte-check/svelte-kit/types/src/routes/$types.js'"),
            "sibling $types import not re-anchored into the mirror: {got}"
        );
    }

    #[test]
    fn ancestor_dollar_types_chains_re_anchor_into_the_mirror() {
        // A deeper route reaching two levels up stays inside the
        // mirror too — depth within the generated tree is irrelevant,
        // only which tree the resolved path lands in.
        let dir = PathBuf::from("/ws/.svelte-kit/types/src/routes/foo/bar");
        let input = "type X = import('../../$types.js').LayoutData;";
        let got = rewrite_from(input, &dir);
        assert!(
            got.contains("'/ws/.svelte-check/svelte-kit/types/src/routes/$types.js'"),
            "ancestor $types import not re-anchored into the mirror: {got}"
        );
    }

    #[test]
    fn multibyte_text_survives_byte_identically() {
        // The scanner copies text between quoted runs as slices; a
        // byte-at-a-time copy would decode each UTF-8 continuation
        // byte as its own mangled Latin-1 char.
        let input = "// naïve Präfix — 路由テスト 🚀\ntype Msg = 'Grüße';";
        assert_eq!(rewrite(input), input);
    }

    #[test]
    fn rewrites_every_chain_in_a_file() {
        let input = "import('../../../../../src/routes/a/+page.js'); import('../../../../../src/routes/b/+page.js');";
        let got = rewrite(input);
        assert!(got.contains("svelte/src/routes/a/+page.js"), "{got}");
        assert!(got.contains("svelte/src/routes/b/+page.js"), "{got}");
    }

    #[test]
    fn bare_package_specifiers_are_left_alone() {
        let input = "import type { Foo } from '@sveltejs/kit';";
        assert_eq!(rewrite(input), input);
    }

    #[test]
    fn non_specifier_quoted_text_is_left_alone() {
        // A quoted string that isn't a relative path must survive
        // untouched — the scanner rewrites only `./` and `../` starts.
        let input = "type Msg = 'src/routes/not-a-path';";
        assert_eq!(rewrite(input), input);
    }

    #[test]
    fn specifiers_outside_the_workspace_are_left_alone() {
        // A chain escaping above the workspace has no meaningful
        // re-anchor and is left as written rather than guessed at.
        let input = "import('../../../../../../../../outside/thing.js')";
        assert_eq!(rewrite(input), input);
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
