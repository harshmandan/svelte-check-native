//! Invariant tests for the line_map / token_map data that
//! `emit_document` produces.
//!
//! Goal: catch off-by-one bugs in emit's map-building logic by
//! asserting structural invariants on the maps for a battery of
//! hand-written Svelte sources covering common shapes (mustache,
//! `{#each}`, `{@const}`, components, snippets, slots, multi-line
//! expressions). Each invariant is checked with a clear assertion
//! message so a regression points at the broken case.
//!
//! NOT a substitute for the bug-fixture suite — this catches DATA-
//! integrity bugs in maps; bug fixtures catch USER-VISIBLE diagnostic-
//! position drift.
//!
//! These tests run in-process (no binary, no tsgo) so they're fast
//! and let us iterate on map-building changes without the full
//! type-check loop.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;

use svn_emit::{LineMapEntry, TokenMapEntry, emit_document};
use svn_parser::{parse_all_template_runs, parse_sections};

/// Run the full parse → analyze → emit pipeline on a hand-written
/// `.svelte` source and assert structural invariants on the resulting
/// line_map / token_map.
fn assert_map_invariants(source: &str, label: &str) {
    let (doc, parse_errors) = parse_sections(source);
    assert!(
        parse_errors.is_empty(),
        "[{label}] parse errors: {parse_errors:?}"
    );
    let (fragment, frag_errors) = parse_all_template_runs(source, &doc.template.text_runs);
    assert!(
        frag_errors.is_empty(),
        "[{label}] fragment errors: {frag_errors:?}"
    );
    let summary = svn_analyze::walk_template(&fragment, source);
    let path = PathBuf::from("test.svelte");
    let out = emit_document(&doc, &fragment, &summary, &path);

    let source_len = source.len() as u32;
    let overlay_len = out.typescript.len() as u32;
    let source_lines = count_lines(source);
    let overlay_lines = count_lines(&out.typescript);

    // ---- LineMapEntry invariants ----
    //
    // The entry shape is `(overlay_start_line, overlay_end_line,
    // source_start_line)` — the source-side end line is implicit
    // (overlay span equals source span by construction, since each
    // entry covers a verbatim source run). The check therefore
    // validates: 1-based, start ≤ end, overlay range fits within the
    // emit, source line fits the source plus one (entries can land at
    // the EOF row).
    for (i, e) in out.line_map.iter().enumerate() {
        let LineMapEntry {
            overlay_start_line,
            overlay_end_line,
            source_start_line,
        } = *e;
        assert!(
            overlay_start_line >= 1,
            "[{label}] line_map[{i}] overlay_start_line={overlay_start_line} must be ≥ 1 (1-based)"
        );
        assert!(
            overlay_start_line <= overlay_end_line,
            "[{label}] line_map[{i}] overlay_start_line={overlay_start_line} > overlay_end_line={overlay_end_line}"
        );
        assert!(
            source_start_line >= 1,
            "[{label}] line_map[{i}] source_start_line={source_start_line} must be ≥ 1 (1-based)"
        );
        assert!(
            overlay_end_line <= overlay_lines + 1,
            "[{label}] line_map[{i}] overlay_end_line={overlay_end_line} > overlay_lines+1={}",
            overlay_lines + 1
        );
        // Source line must fit. The entry's source span is implied to
        // be the same length as the overlay span; the start needs at
        // most `(overlay_end - overlay_start)` more lines available
        // beyond it in the source.
        let span = overlay_end_line - overlay_start_line;
        let implied_source_end = source_start_line + span;
        assert!(
            implied_source_end <= source_lines + 1,
            "[{label}] line_map[{i}] implied source_end_line={implied_source_end} > source_lines+1={} (entry: {e:?}, source has {source_lines} lines)",
            source_lines + 1
        );
    }
    // line_map entries should be sorted by overlay_start_line and not
    // overlap (each verbatim run is a distinct chunk).
    for pair in out.line_map.windows(2) {
        let a = &pair[0];
        let b = &pair[1];
        assert!(
            a.overlay_end_line <= b.overlay_start_line,
            "[{label}] line_map entries overlap on overlay axis: {a:?} then {b:?}"
        );
    }

    // ---- TokenMapEntry invariants ----
    for (i, e) in out.token_map.iter().enumerate() {
        let TokenMapEntry {
            overlay_byte_start,
            overlay_byte_end,
            source_byte_start,
            source_byte_end,
        } = *e;
        assert!(
            overlay_byte_start <= overlay_byte_end,
            "[{label}] token_map[{i}] overlay range inverted: start={overlay_byte_start} end={overlay_byte_end}"
        );
        assert!(
            overlay_byte_end <= overlay_len,
            "[{label}] token_map[{i}] overlay_byte_end={overlay_byte_end} > overlay_len={overlay_len}"
        );
        assert!(
            source_byte_start <= source_byte_end,
            "[{label}] token_map[{i}] source range inverted: start={source_byte_start} end={source_byte_end}"
        );
        assert!(
            source_byte_end <= source_len,
            "[{label}] token_map[{i}] source_byte_end={source_byte_end} > source_len={source_len}"
        );
    }

    // ---- overlay_line_starts invariants ----
    //
    // `compute_line_starts` returns a list whose last entry is the
    // EOF byte offset (a sentinel that lets consumers clamp past-EOF
    // positions). When the buffer ends with `\n`, the EOF offset
    // equals the offset right after the last newline — duplicates
    // at the tail are EXPECTED and load-bearing. So the invariant is
    // monotonic non-decreasing, not strictly increasing.
    for pair in out.overlay_line_starts.windows(2) {
        assert!(
            pair[0] <= pair[1],
            "[{label}] overlay_line_starts not monotonic non-decreasing: {pair:?}"
        );
    }
    if let Some(last) = out.overlay_line_starts.last() {
        assert!(
            *last <= overlay_len,
            "[{label}] last overlay_line_start={last} > overlay_len={overlay_len}"
        );
    }
}

fn count_lines(s: &str) -> u32 {
    let nl = s.bytes().filter(|&b| b == b'\n').count() as u32;
    if s.is_empty() {
        0
    } else if s.ends_with('\n') {
        nl
    } else {
        nl + 1
    }
}

// ---- Test cases ----

#[test]
fn invariants_minimal_template() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    const x = 1;\n</script>\n\n<p>{x}</p>\n",
        "minimal_template",
    );
}

#[test]
fn invariants_each_block() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    const items = [1, 2];\n</script>\n\n{#each items as item}\n    <p>{item}</p>\n{/each}\n",
        "each_block",
    );
}

#[test]
fn invariants_at_const() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    const obj = { a: 1 };\n</script>\n\n{@const v = obj.a * 2}\n<p>{v}</p>\n",
        "at_const",
    );
}

#[test]
fn invariants_multiline_interpolation() {
    // {expr} that spans multiple lines — historically the source-map
    // drift sweet spot. Ensures each chunk's line_map / token_map
    // entries respect the line-count invariant.
    assert_map_invariants(
        "<script lang=\"ts\">\n    const arr: number[] = [];\n</script>\n\n{arr\n    .map((x) => x * 2)\n    .join(',')}\n",
        "multiline_interpolation",
    );
}

#[test]
fn invariants_nested_blocks() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    const items = [{ ok: true }];\n</script>\n\n{#each items as item}\n    {#if item.ok}\n        <p>{item.ok}</p>\n    {/if}\n{/each}\n",
        "nested_blocks",
    );
}

#[test]
fn invariants_snippet_block() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    let x = 0;\n</script>\n\n{#snippet greet(name: string)}\n    <p>Hello {name}, x={x}</p>\n{/snippet}\n",
        "snippet_block",
    );
}

#[test]
fn invariants_at_html() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    const html = '<b>x</b>';\n</script>\n\n{@html html}\n",
        "at_html",
    );
}

#[test]
fn invariants_at_render() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    import type { Snippet } from 'svelte';\n    let { children }: { children: Snippet } = $props();\n</script>\n\n{@render children()}\n",
        "at_render",
    );
}

#[test]
fn invariants_at_debug() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    const a = 1;\n    const b = 2;\n</script>\n\n{@debug a, b}\n",
        "at_debug",
    );
}

#[test]
fn invariants_dom_directive_animate() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    import { flip } from 'svelte/animate';\n</script>\n\n<div animate:flip>x</div>\n",
        "dom_directive_animate",
    );
}

#[test]
fn invariants_use_action() {
    assert_map_invariants(
        "<script lang=\"ts\">\n    import type { Action } from 'svelte/action';\n    const enhance: Action = () => {};\n</script>\n\n<form use:enhance>x</form>\n",
        "use_action",
    );
}

#[test]
fn invariants_empty_template() {
    // Edge case: script-only file.
    assert_map_invariants(
        "<script lang=\"ts\">\n    const x = 1;\n</script>\n",
        "empty_template",
    );
}

#[test]
fn invariants_no_script() {
    // Edge case: template-only file (no script section).
    assert_map_invariants("<p>hello</p>\n", "no_script");
}

#[test]
fn invariants_empty_file() {
    assert_map_invariants("", "empty_file");
}
