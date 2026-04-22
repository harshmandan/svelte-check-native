//! `<!-- svelte-ignore CODE [, CODE, …] -->` extractor.
//!
//! Mirrors `svelte/src/compiler/utils/extract_svelte_ignore.js` —
//! including the 9 legacy→new rename entries + runes-mode comma
//! parsing + lax non-runes-mode parsing.

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::ast::{Comment, Node};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// Legacy dashed codes → new underscore codes. Static table copied
/// from upstream `extract_svelte_ignore.js:7-17`.
const LEGACY_RENAMES: &[(&str, &str)] = &[
    (
        "non-top-level-reactive-declaration",
        "reactive_declaration_invalid_placement",
    ),
    (
        "module-script-reactive-declaration",
        "reactive_declaration_module_script",
    ),
    ("empty-block", "block_empty"),
    ("avoid-is", "attribute_avoid_is"),
    ("invalid-html-attribute", "attribute_invalid_property_name"),
    ("a11y-structure", "a11y_figcaption_parent"),
    ("illegal-attribute-character", "attribute_illegal_colon"),
    ("invalid-rest-eachblock-binding", "bind_invalid_each_rest"),
    ("unused-export-let", "export_let_unused"),
];

/// Walk the siblings of `target` in `nodes`, collecting any
/// `svelte-ignore` codes found in preceding `Comment` nodes (with
/// intervening whitespace `Text` allowed). Stops at any non-
/// Comment / non-Text sibling.
///
/// Returns the deduplicated list of ignore codes as SmolStr suitable
/// for pushing into `LintContext::ignore_stack`.
pub fn collect_preceding_comment_ignores(
    nodes: &[Node],
    target: &Node,
    ctx: &mut LintContext<'_>,
) -> Vec<SmolStr> {
    let mut result: Vec<SmolStr> = Vec::new();
    let Some(idx) = nodes.iter().position(|n| std::ptr::eq(n, target)) else {
        return result;
    };
    if idx == 0 {
        return result;
    }

    // Walk backwards collecting codes until we see a non-Comment
    // non-Text sibling.
    for i in (0..idx).rev() {
        match &nodes[i] {
            Node::Comment(c) => {
                for code in extract_from_comment(c, ctx) {
                    if !result.contains(&code) {
                        result.push(code);
                    }
                }
            }
            Node::Text(t) => {
                // Only whitespace-only text continues the chain.
                if !t.content.chars().all(char::is_whitespace) {
                    break;
                }
            }
            _ => break,
        }
    }
    result
}

/// Extract `svelte-ignore CODE, CODE` codes from one comment, and
/// emit `legacy_code` / `unknown_code` for tokens that don't match a
/// known warning code (runes mode only — upstream matches this).
fn extract_from_comment(c: &Comment, ctx: &mut LintContext<'_>) -> Vec<SmolStr> {
    let trimmed = c.data.trim_start();
    let Some(rest) = trimmed.strip_prefix("svelte-ignore") else {
        return Vec::new();
    };
    let ws = match rest.chars().next() {
        Some(ch) if ch.is_whitespace() => ch,
        _ => return Vec::new(),
    };
    let rest_after_ws = &rest[ws.len_utf8()..];

    // Byte offset of `rest_after_ws` inside the source — the Comment
    // body starts at `c.range.start + 4` (`<!--` is 4 bytes).
    let comment_body_start = c.range.start + 4;
    // Offset within comment body where the trimmed "svelte-ignore "
    // prefix ends and the code list begins.
    let prefix_len = (c.data.len() - rest_after_ws.len()) as u32;
    let tokens_start = comment_body_start + prefix_len;

    parse_ignore_codes_emit(rest_after_ws, ctx, tokens_start)
}

fn parse_ignore_codes_emit(
    rest: &str,
    ctx: &mut LintContext<'_>,
    rest_base_offset: u32,
) -> Vec<SmolStr> {
    let mut out = Vec::new();
    if ctx.runes {
        // Split on commas. We walk the string manually to track byte
        // offsets for the diagnostic ranges.
        let mut i = 0usize;
        let bytes = rest.as_bytes();
        while i < bytes.len() {
            // Skip leading whitespace inside the segment.
            let seg_start = i;
            while i < bytes.len() && bytes[i] != b',' {
                i += 1;
            }
            let segment = &rest[seg_start..i];
            if i < bytes.len() {
                i += 1; // consume comma
            }
            // Extract the leading identifier from segment.
            let leading_ws: usize = segment
                .chars()
                .take_while(|c| c.is_whitespace())
                .map(|c| c.len_utf8())
                .sum();
            let tok_source = &segment[leading_ws..];
            let token_len: usize = tok_source
                .chars()
                .take_while(|c| {
                    c.is_ascii_alphanumeric() || *c == '_' || *c == '-' || *c == '$'
                })
                .map(|c| c.len_utf8())
                .sum();
            if token_len == 0 {
                continue;
            }
            let code = &tok_source[..token_len];
            let token_abs_start =
                rest_base_offset + (seg_start as u32) + (leading_ws as u32);
            let token_abs_end = token_abs_start + token_len as u32;
            if is_known_code(code) {
                out.push(SmolStr::new(code));
                continue;
            }
            // Upstream fires legacy_code / unknown_code but does NOT
            // push the rewritten form into the ignore list — the
            // user has to update their code to the new name for the
            // suppression to take effect.
            let replacement = legacy_rename(code)
                .map(str::to_string)
                .unwrap_or_else(|| code.replace('-', "_"));
            let range = Range::new(token_abs_start, token_abs_end);
            if is_known_code(&replacement) {
                let msg = messages::legacy_code(code, &replacement);
                ctx.emit(Code::legacy_code, msg, range);
            } else {
                let suggestion = fuzzymatch_known_code(code);
                let msg = messages::unknown_code(code, suggestion);
                ctx.emit(Code::unknown_code, msg, range);
            }
        }
    } else {
        // Non-runes lax parsing — no warnings, same legacy-rename
        // pushthrough.
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            while i < bytes.len() && !is_ident_char(bytes[i]) {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            if start == i {
                break;
            }
            let token = &rest[start..i];
            let mapped = legacy_rename(token)
                .map(str::to_string)
                .unwrap_or_else(|| token.replace('-', "_"));
            let sm = SmolStr::new(&mapped);
            let should_also_push_raw = mapped != token;
            if !out.contains(&sm) {
                out.push(sm);
            }
            if should_also_push_raw {
                let raw = SmolStr::new(token);
                if !out.contains(&raw) {
                    out.push(raw);
                }
            }
        }
    }
    out
}

/// Parse the tail of `svelte-ignore ` into a list of codes.
/// - **runes mode**: strict, comma-separated; unknown codes get
///   `legacy_code` / `unknown_code` warnings (Phase A will wire the
///   fire points — stubbed as no-op for now).
/// - **legacy mode**: lax; any word-ish token is a code.
pub fn parse_ignore_codes_public(rest: &str, runes: bool) -> Vec<SmolStr> {
    parse_ignore_codes(rest, runes)
}

fn parse_ignore_codes(rest: &str, runes: bool) -> Vec<SmolStr> {
    let mut out = Vec::new();
    if runes {
        // Split on commas.
        for raw in rest.split(',') {
            let token = raw.trim();
            if token.is_empty() {
                break;
            }
            // Only word-ish chars.
            let code: String = token
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_' || *c == '-' || *c == '$')
                .collect();
            if code.is_empty() {
                break;
            }
            let mapped = if is_known_code(&code) {
                code
            } else if let Some(to) = legacy_rename(&code) {
                to.to_string()
            } else {
                // Unknown — would fire `unknown_code`. For now skip.
                code.replace('-', "_")
            };
            let sm = SmolStr::new(&mapped);
            if !out.contains(&sm) {
                out.push(sm);
            }
        }
    } else {
        // Lax: any word-ish run.
        let bytes = rest.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            // Skip non-word.
            while i < bytes.len() && !is_ident_char(bytes[i]) {
                i += 1;
            }
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            if start == i {
                break;
            }
            let token = &rest[start..i];
            let mapped = legacy_rename(token)
                .map(str::to_string)
                .unwrap_or_else(|| token.replace('-', "_"));
            let sm = SmolStr::new(&mapped);
            let should_also_push_raw = mapped != token;
            if !out.contains(&sm) {
                out.push(sm);
            }
            // Also push the raw form so user's legacy `<!-- svelte-ignore a11y-structure -->`
            // ignores BOTH the legacy form (for upstream parity) and
            // the rewritten form.
            if should_also_push_raw {
                let raw = SmolStr::new(token);
                if !out.contains(&raw) {
                    out.push(raw);
                }
            }
        }
    }
    out
}

fn is_ident_char(b: u8) -> bool {
    (b as char).is_alphanumeric() || matches!(b, b'_' | b'-' | b'$')
}

/// Closest known code name to `input` via Levenshtein similarity.
/// Returns `None` when nothing clears the 0.7 threshold (mirrors
/// upstream fuzzymatch.js).
fn fuzzymatch_known_code(input: &str) -> Option<&'static str> {
    let target = input.to_ascii_lowercase();
    let mut best: Option<(f64, &'static str)> = None;
    for c in crate::codes::CODES {
        let sim = lev_similarity(&target, c);
        if sim >= 0.7 && best.map(|(s, _)| sim > s).unwrap_or(true) {
            best = Some((sim, c));
        }
    }
    best.map(|(_, c)| c)
}

fn lev_similarity(a: &str, b: &str) -> f64 {
    let ac: Vec<char> = a.chars().collect();
    let bc: Vec<char> = b.chars().collect();
    if ac.is_empty() && bc.is_empty() {
        return 1.0;
    }
    let max = ac.len().max(bc.len()) as f64;
    let mut prev: Vec<usize> = (0..=bc.len()).collect();
    let mut curr = vec![0usize; bc.len() + 1];
    for i in 1..=ac.len() {
        curr[0] = i;
        for j in 1..=bc.len() {
            let cost = if ac[i - 1] == bc[j - 1] { 0 } else { 1 };
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    1.0 - prev[bc.len()] as f64 / max
}

fn is_known_code(code: &str) -> bool {
    Code::try_from_str(code).is_some()
}

fn legacy_rename(code: &str) -> Option<&'static str> {
    LEGACY_RENAMES
        .iter()
        .find_map(|(from, to)| if *from == code { Some(*to) } else { None })
}
