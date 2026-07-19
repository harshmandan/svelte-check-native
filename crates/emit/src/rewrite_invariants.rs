//! Invariant locks for every in-place script rewrite.
//!
//! The diagnostic position mapper leans on two guarantees:
//!
//! 1. **Line map validity** — the line map records overlay-line →
//!    source-line ranges for the spliced script body. Every rewrite
//!    that changes the body BEFORE or AFTER the splice must preserve
//!    the line count (never insert or delete a newline), otherwise
//!    every diagnostic below the rewrite lands on the wrong source
//!    LINE.
//!
//! 2. **Column recovery** — in-line rewrites shift the columns of
//!    everything to their right on the same line. Two mechanisms
//!    recover them:
//!    - Rewrites applied BEFORE the splice are covered by the
//!      longest-common-prefix/suffix token-map entries the splice
//!      site records (`common_affix_spans` in `lib.rs`): byte-equal
//!      regions map 1:1, only the changed middle degrades to the
//!      line map's column passthrough.
//!    - Rewrites applied AFTER the splice (the `*_in_place` family)
//!      must return their insertions as `(position, length)` pairs so
//!      `EmitBuffer::adjust_token_map_for_insertions` re-anchors the
//!      already-recorded token map. An unreported length change
//!      silently corrupts every later entry.
//!
//! Each test below asserts, on a sample that actually TRIGGERS the
//! rewrite, exactly the invariant that rewrite claims. **Any new
//! in-place rewrite of script content must add a test here** stating
//! which invariant it upholds — a rewrite with no entry in this
//! module has no contract with the position mapper.

use smol_str::SmolStr;
use svn_parser::ScriptLang;

use crate::props_emit::inject_component_props_annotation;
use crate::svelte2tsx_nodes::component_events::rewrite_dispatcher_typing;
use crate::svelte4::compat::{
    denarrow_typed_exported_props_in_place, rewrite_definite_assignment_in_place,
    rewrite_void_sequence_to_array, widen_untyped_exported_props_in_place,
    widen_untyped_exports_jsdoc_in_place,
};
use crate::svelte4::reactive::rewrite_with_touched_names;
use crate::util::blank_dollar_generic_decls;
use crate::{common_affix_spans, process_instance_script_content::split_imports};

fn line_count(s: &str) -> usize {
    s.bytes().filter(|&b| b == b'\n').count()
}

/// A pre-splice rewrite must preserve the line count (line map
/// validity) and must have actually changed the input (otherwise the
/// invariant is asserted vacuously).
fn assert_line_preserving_rewrite(input: &str, output: &str) {
    assert_ne!(input, output, "sample did not trigger the rewrite");
    assert_eq!(
        line_count(input),
        line_count(output),
        "rewrite changed the line count — the line map is now wrong \
         for every diagnostic below the rewrite"
    );
}

/// A post-splice in-place rewrite must report edits that EXACTLY
/// reconstruct the output from the input: interleave input segments
/// with inserted chunks at the reported pre-rewrite positions. Also
/// asserts no inserted chunk contains a newline (line map validity —
/// `adjust_token_map_for_insertions` only re-anchors byte spans).
fn assert_edits_reconstruct(input: &str, output: &str, edits: &[(u32, u32)]) {
    assert_ne!(input, output, "sample did not trigger the rewrite");
    let mut rebuilt = String::with_capacity(output.len());
    let mut in_cursor = 0usize;
    let mut out_cursor = 0usize;
    for &(pos, len) in edits {
        let (pos, len) = (pos as usize, len as usize);
        assert!(pos >= in_cursor, "edits must be ascending");
        let seg = &input[in_cursor..pos];
        rebuilt.push_str(seg);
        out_cursor += seg.len();
        let inserted = &output[out_cursor..out_cursor + len];
        assert!(
            !inserted.contains('\n'),
            "inserted text contains a newline: {inserted:?}"
        );
        rebuilt.push_str(inserted);
        out_cursor += len;
        in_cursor = pos;
    }
    rebuilt.push_str(&input[in_cursor..]);
    assert_eq!(
        rebuilt, output,
        "reported edits do not reconstruct the rewritten buffer — \
         adjust_token_map_for_insertions will mis-anchor the token map"
    );
}

// ---- pre-splice rewrites (line-count preserving; columns recovered
// ---- by the splice site's common-affix token-map entries) ----

#[test]
fn reactive_rewrite_preserves_line_count() {
    // All three `$:` shapes: declaration, re-assignment, block.
    let src = "let count = 0;\n\
               $: doubled = count * 2;\n\
               $: count = doubled - count;\n\
               $: {\n\tconsole.log(count);\n}\n";
    let (out, _) = rewrite_with_touched_names(src, ScriptLang::Ts);
    assert_line_preserving_rewrite(src, &out);
}

#[test]
fn dispatcher_typing_rewrite_preserves_line_count() {
    let src = "import { createEventDispatcher } from 'svelte';\n\
               const dispatch = createEventDispatcher();\n";
    let out = rewrite_dispatcher_typing(src, ScriptLang::Ts);
    assert_line_preserving_rewrite(src, &out);
}

#[test]
fn props_annotation_injection_preserves_line_count() {
    // Insert shape: untyped `$props()` destructure gains
    // `: $$ComponentProps`.
    let src = "const { data } = $props();\ndata;\n";
    let out = inject_component_props_annotation(src, ScriptLang::Ts);
    assert_line_preserving_rewrite(src, &out);

    // Replace shape: a literal annotation is swapped for the alias.
    let src = "const { data }: { data: string } = $props();\ndata;\n";
    let out = inject_component_props_annotation(src, ScriptLang::Ts);
    assert_line_preserving_rewrite(src, &out);
}

#[test]
fn blank_dollar_generic_decls_preserves_length_and_lines() {
    let src = "type T = $$Generic<string>;\nlet x: T;\n";
    let out = blank_dollar_generic_decls(src);
    assert_line_preserving_rewrite(src, &out);
    // Stronger: blanking is length-preserving, so byte positions of
    // everything around the blanked span stay exact.
    assert_eq!(src.len(), out.len(), "blanking must be length-preserving");
}

#[test]
fn split_imports_body_preserves_length_and_lines() {
    let src = "import { writable } from 'svelte/store';\n\
               export const flag = true;\n\
               let store = writable(0);\n";
    let split = split_imports(src, ScriptLang::Ts, false, None);
    assert_ne!(split.body, src, "sample did not trigger any hoisting");
    assert_eq!(
        src.len(),
        split.body.len(),
        "hoist blanking must be length-preserving"
    );
    assert_eq!(line_count(src), line_count(&split.body));
}

// ---- post-splice rewrites (must report exact edits for
// ---- adjust_token_map_for_insertions) ----

#[test]
fn definite_assignment_edits_reconstruct() {
    let input = "let el: HTMLElement;\nlet other: string;\n";
    let mut out = input.to_string();
    let edits = rewrite_definite_assignment_in_place(
        &mut out,
        &[SmolStr::from("el"), SmolStr::from("other")],
    );
    assert_edits_reconstruct(input, &out, &edits);
}

#[test]
fn widen_untyped_exports_edits_reconstruct() {
    let input = "let foo;\nlet bar;\n";
    let mut out = input.to_string();
    let edits = widen_untyped_exported_props_in_place(
        &mut out,
        &[SmolStr::from("foo"), SmolStr::from("bar")],
        None,
    );
    assert_edits_reconstruct(input, &out, &edits);
}

#[test]
fn widen_untyped_exports_jsdoc_edits_reconstruct() {
    let input = "let foo;\nlet bar;\n";
    let mut out = input.to_string();
    let edits = widen_untyped_exports_jsdoc_in_place(
        &mut out,
        &[SmolStr::from("foo"), SmolStr::from("bar")],
        None,
    );
    assert_edits_reconstruct(input, &out, &edits);
}

#[test]
fn denarrow_typed_exports_edits_reconstruct() {
    let input = "let size: string = 'medium';\nsize;\n";
    let mut out = input.to_string();
    let edits = denarrow_typed_exported_props_in_place(&mut out, &[SmolStr::from("size")]);
    assert_edits_reconstruct(input, &out, &edits);
}

#[test]
fn void_sequence_rewrite_accounts_for_length_and_lines() {
    // This rewrite mixes length-preserving replacements (`(` → `[`,
    // `)` → `]`), which need no re-anchoring, with real insertions
    // (the `void ` prefix on `$:` labels). The reconstruct helper
    // can't verify byte-identity across the replacements, so assert
    // the weaker contract the token-map adjustment depends on: the
    // reported edits account for the FULL length delta, and no
    // newline was introduced.
    let input = "void (a, b, c);\n$: (d, e);\n";
    let mut out = input.to_string();
    let edits = rewrite_void_sequence_to_array(&mut out);
    assert_ne!(input, out, "sample did not trigger the rewrite");
    let reported: usize = edits.iter().map(|&(_, len)| len as usize).sum();
    assert_eq!(
        out.len() - input.len(),
        reported,
        "unreported length change corrupts the token map"
    );
    assert_eq!(line_count(input), line_count(&out));
}

// ---- the splice site's column-recovery helper itself ----

#[test]
fn common_affix_spans_full_match_is_one_span() {
    let s = b"const a = 1;";
    assert_eq!(common_affix_spans(s, s), vec![(0, 0, s.len() as u32)]);
}

#[test]
fn common_affix_spans_insertion_maps_both_sides() {
    let src = "let el: T | null = $state(null); const bad: string = 1;";
    let body = "let el: T | null = $state<T | null>(null); const bad: string = 1;";
    let spans = common_affix_spans(src.as_bytes(), body.as_bytes());
    assert_eq!(spans.len(), 2);
    // Prefix covers everything up to the insertion point.
    let prefix_len = "let el: T | null = $state".len() as u32;
    assert_eq!(spans[0], (0, 0, prefix_len));
    // Suffix aligns the shifted tail with its source position.
    let suffix_len = "(null); const bad: string = 1;".len() as u32;
    assert_eq!(
        spans[1],
        (
            body.len() as u32 - suffix_len,
            src.len() as u32 - suffix_len,
            suffix_len
        )
    );
}

#[test]
fn common_affix_spans_never_overlaps_on_repetitive_content() {
    // Suffix is capped at min(len) - prefix so the two spans can't
    // double-map the same source bytes.
    let src = "aaaa";
    let body = "aaaaaa";
    let spans = common_affix_spans(src.as_bytes(), body.as_bytes());
    let total: u32 = spans.iter().map(|&(_, _, len)| len).sum();
    assert!(total <= src.len() as u32);
}

#[test]
fn common_affix_spans_respects_char_boundaries() {
    // The divergence point lands inside a multi-byte char on one
    // side; span edges must be rounded to char boundaries so byte →
    // (line, col) conversion never slices mid-char.
    let src = "let x = 'é';";
    let body = "let x = 'a';";
    for (o, s, len) in common_affix_spans(src.as_bytes(), body.as_bytes()) {
        assert!(src.is_char_boundary(s as usize));
        assert!(src.is_char_boundary((s + len) as usize));
        assert!(body.is_char_boundary(o as usize));
        assert!(body.is_char_boundary((o + len) as usize));
    }
}
