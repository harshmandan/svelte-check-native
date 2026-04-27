//! Diagnostic filters — predicates that recognize false-positive
//! categories we drop in `map_diagnostic`.
//!
//! Most predicates take overlay text + byte offset. The byte offset
//! comes from [`crate::position::overlay_byte_offset`] in the
//! diagnostic mapper.

use std::path::Path;

use crate::cache::CacheLayout;
use crate::output::RawDiagnostic;
use crate::types::{CheckDiagnostic, IGNORE_END_MARKER, IGNORE_START_MARKER};

/// Filter for diagnostics that come from our own overlay tsconfig and
/// represent intentional choices we've made — they're not user-actionable.
///
/// Robust against the path-shape tsgo emits: it formats diagnostic
/// paths relative to its own cwd. We set tsgo's cwd to the workspace
/// in `run_tsgo`, so a relative `raw.file` joins back to the right
/// absolute path. As defense in depth we also accept a match by
/// canonicalized absolute path (handles symlinks like `/var` vs
/// `/private/var` on macOS) and a final ends-with check on the unique
/// `.svelte-check/tsconfig.json` suffix.
pub(crate) fn is_overlay_tsconfig_noise(raw: &RawDiagnostic, layout: &CacheLayout) -> bool {
    let abs = if raw.file.is_absolute() {
        raw.file.clone()
    } else {
        layout.workspace.join(&raw.file)
    };
    if abs == layout.overlay_tsconfig {
        return true;
    }
    if let (Ok(a), Ok(b)) = (
        dunce::canonicalize(&abs),
        dunce::canonicalize(&layout.overlay_tsconfig),
    ) {
        if a == b {
            return true;
        }
    }
    // Last resort: tsgo on some configurations emits the path as
    // workspace-relative even when the overlay was passed absolute.
    // The overlay's basename + parent directory together are unique
    // enough that any path matching both is ours.
    let overlay_name = layout
        .overlay_tsconfig
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let overlay_parent_name = layout
        .overlay_tsconfig
        .parent()
        .and_then(Path::file_name)
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if !overlay_name.is_empty() && !overlay_parent_name.is_empty() {
        let raw_name = raw.file.file_name().and_then(|s| s.to_str()).unwrap_or("");
        let raw_parent_name = raw
            .file
            .parent()
            .and_then(Path::file_name)
            .and_then(|s| s.to_str())
            .unwrap_or("");
        if raw_name == overlay_name && raw_parent_name == overlay_parent_name {
            return true;
        }
    }
    false
}

/// SVELTE-4-COMPAT candidate: suppress TS2695 "Left side of comma
/// operator is unused and has no side effects" on `.svelte` files
/// that specifically trigger the Svelte-4 `$: (a, b, c)` dep-tracking
/// idiom. Upstream svelte-check filters these via
/// `isInReactiveStatement` in
/// `language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:512-543`
/// — only diagnostics whose overlay AST node has a `$:` labeled-
/// statement ancestor get suppressed.
///
/// Historical note: this used to be a blanket drop of ALL TS2695 on
/// `.svelte` files. Empirical survey across our bench fleet found
/// exactly zero legitimate dep-tracking hits the blanket filter
/// silenced that weren't already silenced by emit rewrites, and ONE
/// upstream-matching fire it wrongly suppressed. The blanket filter
/// was removed in favour of this narrower, currently never-fires
/// path. Extend if a future Svelte-4 project surfaces the idiom.
pub(crate) fn is_svelte4_reactive_noop_comma(diag: &CheckDiagnostic) -> bool {
    let _ = diag;
    false
}

/// SVELTE-4-COMPAT: does the overlay text at `offset` start a Svelte-4
/// `$:` reactive-statement label?
///
/// tsgo's TS7028 ("Unused label") points at the **identifier** that
/// names the label — for `$: foo()` that's the `$` character at
/// `offset`, with `:` immediately after. Both ours and upstream emit
/// reactive statements as `;() => { $: <expr> }` (preserves the user's
/// reactive code as a body for type-checking without actually running
/// it), so the structural `$:` is the source of false-positive TS7028s
/// when the user's tsconfig has `allowUnusedLabels: false` (default in
/// strict-mode SvelteKit + threlte tsconfigs).
///
/// `overlay_text[offset]` must be `$` and `overlay_text[offset+1]` must
/// be `:`. The `$` identifier is exactly one byte; tolerate optional
/// whitespace between `$` and `:` purely defensively (Svelte's compiler
/// rejects whitespace there, but our future emit might add it).
pub(crate) fn is_overlay_dollar_reactive_label(overlay: &str, offset: u32) -> bool {
    let bytes = overlay.as_bytes();
    let off = offset as usize;
    if bytes.get(off) != Some(&b'$') {
        return false;
    }
    let mut i = off + 1;
    while bytes.get(i).is_some_and(|b| b.is_ascii_whitespace()) {
        i += 1;
    }
    bytes.get(i) == Some(&b':')
}

/// Does the overlay text at `offset` start a quoted attribute key
/// of the form a `createElement(...)` literal would emit (e.g.
/// `"on:click"`, `"class"`, `"id"`)?
///
/// Used to filter TS1117/TS2300 duplicate-key diagnostics on element
/// attribute names; mirrors upstream svelte-check's
/// `isAttributeName(node, 'Element') || isEventHandler(node, 'Element')`
/// filter at `DiagnosticsProvider.ts:366-371`. Less precise (no AST),
/// but in practice covers the same patterns: Svelte's parser rejects
/// duplicate static attributes at compile time, so the only overlay
/// duplicates that reach the type-checker come from the
/// `<el on:click={fn} on:click>` (handle + forward) idiom or from
/// spread-plus-attribute combinations.
pub(crate) fn is_overlay_attribute_key(overlay: &str, offset: u32) -> bool {
    let bytes = overlay.as_bytes();
    let off = offset as usize;
    // tsgo's TS1117/TS2300 sometimes points at the opening `"`, sometimes
    // at the first character INSIDE the quotes (the duplicate identifier
    // itself). Walk backwards through valid attribute-name chars to
    // find the opening quote.
    let mut start = off;
    while start > 0
        && bytes
            .get(start)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_' | b'$'))
    {
        if start == 0 {
            break;
        }
        start -= 1;
    }
    if bytes.get(start) != Some(&b'"') {
        return false;
    }
    let mut i = start + 1;
    while let Some(&b) = bytes.get(i) {
        if b == b'"' {
            // Closing quote; check that `:` follows (with optional ws).
            let mut j = i + 1;
            while bytes.get(j).is_some_and(|c| c.is_ascii_whitespace()) {
                j += 1;
            }
            return bytes.get(j) == Some(&b':');
        }
        if !(b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_' | b'$')) {
            return false;
        }
        i += 1;
    }
    false
}

/// Check whether `offset` falls inside any `(start, end)` range in
/// `regions`. Linear scan; regions are typically few per file.
pub(crate) fn is_in_ignore_region(regions: &[(u32, u32)], offset: u32) -> bool {
    regions
        .iter()
        .any(|&(start, end)| offset >= start && offset < end)
}

/// Scan `overlay_text` for [`IGNORE_START_MARKER`] / [`IGNORE_END_MARKER`]
/// pairs and return their byte-offset ranges in the overlay.
///
/// Each `ignore_start` pairs with the NEXT `ignore_end` (mirrors
/// upstream's `isInGeneratedCode` pairing semantics). A stray
/// unmatched `ignore_start` with no subsequent `ignore_end` extends
/// to `overlay_text.len()` — equivalent to "everything after this
/// marker is scaffolding". Empty result when the overlay has no
/// markers.
pub fn scan_ignore_regions(overlay_text: &str) -> Vec<(u32, u32)> {
    let bytes = overlay_text.as_bytes();
    let start_marker = IGNORE_START_MARKER.as_bytes();
    let end_marker = IGNORE_END_MARKER.as_bytes();
    let mut regions: Vec<(u32, u32)> = Vec::new();
    let mut cursor: usize = 0;
    while let Some(rel) = find_subslice(&bytes[cursor..], start_marker) {
        let start = cursor + rel;
        // Region begins AFTER the start marker (so the marker itself
        // is tolerated — no diagnostic can legitimately originate
        // inside a comment).
        let region_start = start + start_marker.len();
        let after_start = region_start;
        let end = match find_subslice(&bytes[after_start..], end_marker) {
            Some(rel_end) => after_start + rel_end,
            None => bytes.len(),
        };
        regions.push((region_start as u32, end as u32));
        cursor = end + end_marker.len().min(bytes.len() - end);
    }
    regions
}

/// `memmem`-style byte-slice search. Rust stdlib doesn't expose this
/// for byte slices so we roll a small one. Linear in haystack size,
/// which is fine for overlay files (~hundreds of KB at most).
fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}
