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

/// TypeScript compiler-option validation codes. Every diagnostic in
/// this family is about an option the USER declared (unknown / removed /
/// deprecated / removed-value / bundler-module mismatch) — our overlay
/// inherits the user's tsconfig via `extends` and never sets a removed
/// or deprecated option itself, so when one of these lands on the
/// overlay tsconfig it is inherited, user-caused, and upstream
/// `svelte-check --tsgo` surfaces it. We surface it too. See
/// [`is_overlay_tsconfig_noise`].
///
/// - `5023` unknown compiler option
/// - `5095` `moduleResolution: bundler` requires a compatible `module`
/// - `5101` option deprecated
/// - `5102` option removed (e.g. `baseUrl`, `outFile`)
/// - `5107` / `5108` / `5109` removed value for module / target /
///   moduleResolution
const USER_CONFIG_OPTION_CODES: &[u32] = &[5023, 5095, 5101, 5102, 5107, 5108, 5109];

/// Filter for diagnostics that come from our own overlay tsconfig and
/// are artifacts of our overlay's structure rather than user-actionable
/// config errors.
///
/// The overlay `extends` the user's tsconfig, so any compiler-option
/// validation error (`USER_CONFIG_OPTION_CODES`) that tsgo attributes to
/// the overlay is actually inherited from the user's own tsconfig —
/// upstream `svelte-check --tsgo` surfaces those (its overlay extends the
/// user config the same way and it applies no overlay filtering), so we
/// surface them too. We only drop OTHER overlay-tsconfig diagnostics,
/// which are structural artifacts of our richer overlay (generated files
/// listed in `files`, rewritten `paths`/`rootDirs`) that upstream's
/// leaner overlay never produces and that aren't user-actionable.
///
/// Robust against the path-shape tsgo emits: it formats diagnostic
/// paths relative to its own cwd. We set tsgo's cwd to the workspace
/// in `run_tsgo`, so a relative `raw.file` joins back to the right
/// absolute path. As defense in depth we also accept a match by
/// canonicalized absolute path (handles symlinks like `/var` vs
/// `/private/var` on macOS) and a final ends-with check on the unique
/// `.svelte-check/tsconfig.json` suffix.
pub(crate) fn is_overlay_tsconfig_noise(raw: &RawDiagnostic, layout: &CacheLayout) -> bool {
    if !is_on_overlay_tsconfig(raw, layout) {
        return false;
    }
    // Inherited user-config-option errors surface (parity with
    // upstream); every other overlay-attributed diagnostic is our own
    // structural noise and drops.
    !USER_CONFIG_OPTION_CODES.contains(&raw.code)
}

/// True when `raw` is attributed to our overlay `tsconfig.json`.
fn is_on_overlay_tsconfig(raw: &RawDiagnostic, layout: &CacheLayout) -> bool {
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
///
/// The text scan alone can't tell a synthesized attribute key from a
/// quoted key the user wrote in their `<script>` — the caller in
/// `map_diagnostic` supplies that context by only invoking this on
/// positions with no line-map coverage (i.e. outside verbatim user
/// code).
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

/// Does the diagnostic at `offset` fall inside an
/// `__svn_ensure_transition(...)` call? Used to drop TS2554
/// "Expected N arguments" — emit wraps every `transition:` /
/// `in:` / `out:` directive call in `__svn_ensure_transition(...)`
/// to give tsgo a typed signature, but the inner user function
/// (e.g. `myTransition(node, params, context)`) declares the
/// optional 3rd `_context` parameter as required and tsgo fires
/// 2554 because we only pass 2 args at the synthetic call site.
/// Svelte's transition runtime supplies the 3rd arg at runtime —
/// the user's source is correct, the synthetic 2-arg call site is
/// the artefact.
///
/// Mirrors upstream svelte-check's `expectedTransitionThirdArgument`
/// filter at
/// `language-tools/packages/language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:663-705`
/// (and the typescript-go provider's variant at
/// `plugins/typescript-go/features/DiagnosticsProvider.ts:1199-1230`).
/// The upstream filter consults the language service to confirm the
/// inner call's signature has exactly 3 non-optional parameters. When
/// no language service is available upstream falls back to matching the
/// diagnostic message text — the substring ` 3`, i.e. "Expected 3
/// arguments". We have no TS language service in our pipeline, so the
/// caller mirrors that no-language-service fallback: it pairs
/// [`is_expected_three_arguments_message`] with this structural origin
/// check. The check here only confirms the diagnostic originates
/// inside the wrapper — if the bytes immediately preceding `offset`
/// (after walking back through identifier characters) end with
/// `__svn_ensure_transition(`. The wrapper only wraps user-supplied
/// transition function calls, so the false-positive surface is narrow.
pub(crate) fn is_overlay_in_ensure_transition_call(overlay: &str, offset: u32) -> bool {
    const PREFIX: &[u8] = b"__svn_ensure_transition(";
    let bytes = overlay.as_bytes();
    let mut cursor = offset as usize;
    if cursor > bytes.len() {
        return false;
    }
    // Walk back through any identifier / whitespace characters
    // to find the start of the inner callee identifier. tsgo's
    // TS2554 may point at the function name (TypeScript >=5.4) or
    // at the open paren of the inner call.
    while cursor > 0 {
        let prev = bytes[cursor - 1];
        if prev.is_ascii_alphanumeric() || prev == b'_' || prev == b'$' {
            cursor -= 1;
        } else {
            break;
        }
    }
    // Skip optional whitespace between the wrapper's `(` and the
    // inner identifier (cosmetic — emit doesn't insert any, but
    // future-proof against a formatter run).
    while cursor > 0 && bytes[cursor - 1].is_ascii_whitespace() {
        cursor -= 1;
    }
    if cursor < PREFIX.len() {
        return false;
    }
    &bytes[cursor - PREFIX.len()..cursor] == PREFIX
}

/// Does a TS2554 message describe the 3-argument transition contract
/// (`Expected 3 arguments, but got 2.`)?
///
/// Mirrors upstream's no-language-service fallback in
/// `expectedTransitionThirdArgument` verbatim — a `' 3'` substring
/// match on the flattened message. The synthetic wrapper call site
/// always passes exactly 2 args, so "but got 3" can never occur there
/// and the loose substring cannot false-match. A transition function
/// with 4+ required params produces "Expected 4 arguments, but got 2."
/// — no ` 3` — so its genuine arity error surfaces, matching the
/// typescript-go provider's exactly-3-non-optional-params signature
/// check.
pub(crate) fn is_expected_three_arguments_message(message: &str) -> bool {
    message.contains(" 3")
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

/// True when the compacted attribute span carries a `lang` attribute
/// set to `pug`. The span has already had whitespace squeezed out, so a
/// genuine `lang` attribute is preceded either by the span start or by a
/// boundary byte (`>`, `/`, a quote, or another attribute's value). We
/// reject a match whose preceding byte is an attribute-name character
/// (`[A-Za-z0-9:_-]`) so `data-lang="pug"` / `xlang="pug"` don't trip the
/// suppression while `lang="pug"` and `foo="x"lang="pug"` still do.
fn lang_attr_is_pug(attrs_compact: &str) -> bool {
    let b = attrs_compact.as_bytes();
    for pat in ["lang=\"pug\"", "lang='pug'", "lang=pug"] {
        let mut from = 0;
        while let Some(rel) = attrs_compact[from..].find(pat) {
            let idx = from + rel;
            let boundary_ok = idx == 0
                || !matches!(b[idx - 1],
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b':' | b'_' | b'-');
            if boundary_ok {
                return true;
            }
            from = idx + 1;
        }
    }
    false
}

/// Scan `source_text` for top-level `<template lang="pug">…</template>`
/// container ranges. Mirrors upstream's `extractTemplateTag` +
/// `isRangeInTag(range, document.templateInfo)` filter at
/// `language-server/src/plugins/typescript/features/DiagnosticsProvider.ts:391-401`,
/// gated on `usesPug = document.getLanguageAttribute('template') ===
/// 'pug'`.
///
/// Each returned `(start, end)` is the byte range of the entire
/// `<template ...>...</template>` container — diagnostics whose source
/// position falls inside drop in `map_diagnostic`, with `6133`
/// (NEVER_READ) and `6192` / `6196` (ALL_IMPORTS_UNUSED) as exceptions
/// that always surface (matching upstream's `isNoPugFalsePositive`).
///
/// The scan is intentionally narrow: it only matches a top-level
/// `<template>` element and only when the `lang` attr is literally
/// `pug`. Other template-tag idioms (`lang="markup"`, no `lang`,
/// custom `lang` values) never produce a suppression range — so a
/// stray `<template>` in the middle of a component's markup still
/// type-checks normally.
pub fn scan_pug_template_ranges(source_text: &str) -> Vec<(u32, u32)> {
    let bytes = source_text.as_bytes();
    let mut out: Vec<(u32, u32)> = Vec::new();
    let mut cursor: usize = 0;
    let open = b"<template";
    let close = b"</template>";
    while let Some(rel) = find_subslice(&bytes[cursor..], open) {
        let tag_start = cursor + rel;
        let after_open = tag_start + open.len();
        // Reject `<templateX...` (identifier continuation) — only
        // accept `<template ` / `<template>` / `<template/`.
        let next = bytes.get(after_open).copied();
        if !matches!(
            next,
            Some(b' ') | Some(b'\t') | Some(b'\n') | Some(b'\r') | Some(b'>') | Some(b'/')
        ) {
            cursor = after_open;
            continue;
        }
        let Some(rel_gt) = bytes[after_open..].iter().position(|&b| b == b'>') else {
            break;
        };
        let open_end = after_open + rel_gt + 1;
        let attrs = &source_text[after_open..open_end - 1];
        let attrs_compact: String = attrs.split_whitespace().collect::<Vec<_>>().join("");
        let is_pug = lang_attr_is_pug(&attrs_compact);
        if !is_pug {
            cursor = open_end;
            continue;
        }
        let Some(rel_close) = find_subslice(&bytes[open_end..], close) else {
            out.push((tag_start as u32, bytes.len() as u32));
            break;
        };
        let close_end = open_end + rel_close + close.len();
        out.push((tag_start as u32, close_end as u32));
        cursor = close_end;
    }
    out
}

/// True when `byte_offset` falls inside any pug-template container
/// range. Used to drop diagnostics inside `<template lang="pug">`
/// bodies (mirrors upstream's `isNoPugFalsePositive`).
pub fn is_in_pug_template(ranges: &[(u32, u32)], byte_offset: u32) -> bool {
    ranges
        .iter()
        .any(|&(start, end)| byte_offset >= start && byte_offset < end)
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
