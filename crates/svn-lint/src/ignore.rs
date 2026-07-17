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

/// Runtime warning codes that upstream additionally accepts in
/// `<!-- svelte-ignore X -->`. Mirrors `IGNORABLE_RUNTIME_WARNINGS` in
/// `svelte/src/constants.js`. Our generated `Code` enum only covers
/// compile warnings (the `messages/compile-warnings/*.md` source), so
/// this list is the second half of upstream's `codes` union in
/// `extract_svelte_ignore.js:20`.
const IGNORABLE_RUNTIME_WARNINGS: &[&str] = &[
    "await_waterfall",
    "await_reactivity_loss",
    "state_snapshot_uncloneable",
    "binding_property_non_reactive",
    "hydration_attribute_changed",
    "hydration_html_changed",
    "ownership_invalid_binding",
    "ownership_invalid_mutation",
];

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

/// Walk the siblings preceding index `idx` in `nodes`, collecting any
/// `svelte-ignore` codes found in preceding `Comment` nodes (with
/// intervening `Text` allowed — whitespace or not). Stops at any
/// non-Comment / non-Text sibling. Mirrors the upstream analyze
/// visitor's backward walk (`else if (prev.type !== 'Text') break`),
/// where every Text node continues the chain.
///
/// Returns the deduplicated list of ignore codes as SmolStr suitable
/// for pushing into `LintContext::ignore_stack`.
pub fn collect_preceding_comment_ignores(
    nodes: &[Node],
    idx: usize,
    ctx: &mut LintContext<'_>,
) -> Vec<SmolStr> {
    let mut result: Vec<SmolStr> = Vec::new();
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
            Node::Text(_) => {}
            _ => break,
        }
    }
    result
}

/// Extract `svelte-ignore CODE, CODE` codes from one comment, and
/// emit `legacy_code` / `unknown_code` for tokens that don't match a
/// known warning code (runes mode only — upstream matches this).
fn extract_from_comment(c: &Comment, ctx: &mut LintContext<'_>) -> Vec<SmolStr> {
    let source = ctx.source;
    let data = c.data_range.slice(source);
    let trimmed = data.trim_start();
    let Some(rest) = trimmed.strip_prefix("svelte-ignore") else {
        return Vec::new();
    };
    let ws = match rest.chars().next() {
        Some(ch) if ch.is_whitespace() => ch,
        _ => return Vec::new(),
    };
    let rest_after_ws = &rest[ws.len_utf8()..];

    // Byte offset of `rest_after_ws` inside the source — the Comment
    // body starts after the `<!--` delimiter.
    let comment_body_start = c.data_range.start;
    // Offset within comment body where the trimmed "svelte-ignore "
    // prefix ends and the code list begins.
    let prefix_len = (data.len() - rest_after_ws.len()) as u32;
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
        // Mirrors upstream's `/([\w$-]+)(,)?/gm` loop: each token is
        // processed (known → suppression list, unknown → warning),
        // then parsing STOPS at the first token not immediately
        // followed by a comma — everything after it is prose.
        for (start, token, followed_by_comma) in runes_ignore_tokens(rest) {
            if is_known_code(token) {
                out.push(SmolStr::new(token));
            } else {
                // Upstream fires legacy_code / unknown_code but does
                // NOT push the rewritten form into the ignore list —
                // the user has to update their code to the new name
                // for the suppression to take effect.
                let token_abs_start = rest_base_offset + start as u32;
                let token_abs_end = token_abs_start + token.len() as u32;
                let replacement = legacy_rename(token)
                    .map(str::to_string)
                    .unwrap_or_else(|| token.replace('-', "_"));
                let range = Range::new(token_abs_start, token_abs_end);
                if is_known_code(&replacement) {
                    let msg = messages::legacy_code(token, &replacement);
                    ctx.emit(Code::legacy_code, msg, range);
                } else {
                    let suggestion = fuzzymatch_known_code(token);
                    let msg = messages::unknown_code(token, suggestion);
                    ctx.emit(Code::unknown_code, msg, range);
                }
            }
            if !followed_by_comma {
                break;
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
            let raw = SmolStr::new(token);
            if !out.contains(&raw) {
                out.push(raw);
            }
            if !is_known_code(token) {
                let mapped = legacy_rename(token)
                    .map(str::to_string)
                    .unwrap_or_else(|| token.replace('-', "_"));
                if is_known_code(&mapped) {
                    let sm = SmolStr::new(&mapped);
                    if !out.contains(&sm) {
                        out.push(sm);
                    }
                }
            }
        }
    }
    out
}

/// Parse the tail of `svelte-ignore ` into the list of suppression
/// codes. Builds the code list only — it does NOT emit any diagnostics.
/// The `legacy_code` / `unknown_code` warnings for unknown or renamed
/// codes are emitted separately by `parse_ignore_codes_emit`.
/// - **runes mode**: strict, comma-separated.
/// - **legacy mode**: lax; any word-ish token is a code.
pub fn parse_ignore_codes_public(rest: &str, runes: bool) -> Vec<SmolStr> {
    parse_ignore_codes(rest, runes)
}

fn parse_ignore_codes(rest: &str, runes: bool) -> Vec<SmolStr> {
    let mut out = Vec::new();
    if runes {
        // Must produce the SAME list as `parse_ignore_codes_emit`'s
        // runes path (the authoritative one run during linting): each
        // token is considered in turn, only KNOWN codes enter the
        // suppression list (legacy/unknown codes are reported by the
        // emit variant but do not suppress until renamed), and parsing
        // stops at the first token not immediately followed by a comma.
        for (_, token, followed_by_comma) in runes_ignore_tokens(rest) {
            if is_known_code(token) {
                let sm = SmolStr::new(token);
                if !out.contains(&sm) {
                    out.push(sm);
                }
            }
            if !followed_by_comma {
                break;
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
            // Push the raw form so a user's legacy
            // `<!-- svelte-ignore a11y-structure -->` ignores the legacy
            // form (for upstream parity), then push the rewritten form too
            // when it is a known code.
            let raw = SmolStr::new(token);
            if !out.contains(&raw) {
                out.push(raw);
            }
            if !is_known_code(token) {
                let mapped = legacy_rename(token)
                    .map(str::to_string)
                    .unwrap_or_else(|| token.replace('-', "_"));
                if is_known_code(&mapped) {
                    let sm = SmolStr::new(&mapped);
                    if !out.contains(&sm) {
                        out.push(sm);
                    }
                }
            }
        }
    }
    out
}

fn is_ident_char(b: u8) -> bool {
    (b as char).is_alphanumeric() || matches!(b, b'_' | b'-' | b'$')
}

/// Index of ALL comments in one script body, powering the
/// leading-comment `svelte-ignore` lookup. Mirrors upstream's estree
/// comment attachment (`phases/1-parse/acorn.js::add_comments`):
/// every queued comment becomes a `leadingComment` of the next node
/// whose start lies after it — no matter how many comments stack up,
/// what separates them from each other, or whether blank lines
/// intervene — EXCEPT a comment consumed as a `trailingComment` of
/// the previous node: same line, separated from it only by `,`, `)`,
/// spaces and tabs (acorn.js:210-212). The analyze walk then honors
/// ignores from ALL leading comments of every node
/// (`2-analyze/index.js:118-131`), which is why suppression works at
/// arbitrary expression positions upstream.
///
/// Reproduced here as a backward chain scan from a node's start:
/// hop over each comment whose gap to the current cursor is
/// whitespace-only, collecting its codes, and stop at a comment that
/// classifies as same-line trailing of whatever precedes it. Plain
/// (non-ignore) comments still chain — that's what makes stacked
/// comment runs work.
pub(crate) struct ScriptComments {
    /// Every comment in the script body, sorted by end offset.
    /// Offsets are script-local, matching oxc spans (delimiters
    /// included).
    entries: Vec<CommentEntry>,
    /// Fast gate: no svelte-ignore comment anywhere → every lookup
    /// answers "no codes" without touching the entries.
    has_ignores: bool,
}

struct CommentEntry {
    start: u32,
    end: u32,
    /// Suppression codes when the comment is a `svelte-ignore`;
    /// empty for every other comment.
    codes: Vec<SmolStr>,
}

impl ScriptComments {
    /// No comments at all — every lookup answers "no codes".
    pub(crate) fn empty() -> Self {
        Self {
            entries: Vec::new(),
            has_ignores: false,
        }
    }

    /// `spans` are the (start, end) byte spans of every comment in
    /// `content`, delimiters included (oxc's `Comment::span`).
    pub(crate) fn build(
        spans: impl Iterator<Item = (u32, u32)>,
        content: &str,
        runes: bool,
    ) -> Self {
        let mut entries: Vec<CommentEntry> = spans
            .filter_map(|(start, end)| {
                let text = content.get(start as usize..end as usize)?;
                let codes = crate::scope_util::strip_comment_delimiters(text)
                    .and_then(|body| {
                        let trimmed = body.trim_start();
                        let rest = trimmed.strip_prefix("svelte-ignore")?;
                        match rest.chars().next() {
                            Some(ch) if ch.is_whitespace() => {
                                Some(parse_ignore_codes(&rest[ch.len_utf8()..], runes))
                            }
                            _ => None,
                        }
                    })
                    .unwrap_or_default();
                Some(CommentEntry { start, end, codes })
            })
            .collect();
        entries.sort_by_key(|e| e.end);
        let has_ignores = entries.iter().any(|e| !e.codes.is_empty());
        Self {
            entries,
            has_ignores,
        }
    }

    pub(crate) fn has_ignores(&self) -> bool {
        self.has_ignores
    }

    /// Suppression codes of the comment run leading a node that
    /// starts at `node_start` (script-local offset into `content`).
    pub(crate) fn leading_ignores(&self, content: &str, node_start: u32) -> Vec<SmolStr> {
        let mut codes: Vec<SmolStr> = Vec::new();
        if !self.has_ignores {
            return codes;
        }
        let bytes = content.as_bytes();
        let mut cursor = (node_start as usize).min(bytes.len());
        // Comments are disjoint, so after hopping to a comment's
        // start every earlier entry ends at or before it — walk the
        // sorted list leftward without re-searching.
        let mut idx = self.entries.partition_point(|e| (e.end as usize) <= cursor);
        while idx > 0 {
            let entry = &self.entries[idx - 1];
            let (start, end) = (entry.start as usize, entry.end as usize);
            if !bytes[end..cursor].iter().all(|b| b.is_ascii_whitespace()) {
                break;
            }
            if is_same_line_trailing(bytes, start) {
                break;
            }
            for c in &entry.codes {
                if !codes.contains(c) {
                    codes.push(c.clone());
                }
            }
            cursor = start;
            idx -= 1;
        }
        codes
    }

    pub(crate) fn has_leading_ignore(&self, content: &str, node_start: u32, code: &str) -> bool {
        self.leading_ignores(content, node_start)
            .iter()
            .any(|c| c.as_str() == code)
    }
}

/// Is the comment starting at `comment_start` a same-line trailing
/// comment of the token before it? Mirrors acorn.js:210-212, which
/// consumes a comment as `trailingComments` of the previous node when
/// only `[,) \t]*` separates them on one line. Scanning backward:
/// hitting the line start first means nothing precedes on this line
/// (leading); otherwise the first blocking byte decides — something
/// that can end an expression (`1`, `'a'`, `]`, `;`, …) means the
/// comment trails it, an opener/operator (`(`, `:`, `=`, …) means the
/// comment leads whatever comes next.
fn is_same_line_trailing(bytes: &[u8], comment_start: usize) -> bool {
    let mut i = comment_start;
    let mut skipped_close_paren = false;
    while i > 0 {
        i -= 1;
        match bytes[i] {
            b'\n' => return false,
            b')' => skipped_close_paren = true,
            b',' | b' ' | b'\t' => {}
            // A `(` reached after skipping a `)` is the opener of a
            // completed call/group (`noop() // c`) — the comment
            // trails that expression. A bare `(` (`take( // c`)
            // means the comment leads the first argument.
            b'(' => return skipped_close_paren,
            b => return is_expression_end_byte(b),
        }
    }
    false
}

fn is_expression_end_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric()
        || b >= 0x80
        || matches!(b, b'_' | b'$' | b'\'' | b'"' | b'`' | b']' | b'}' | b';')
}

/// Byte class of upstream's runes-mode token pattern `[\w$-]` (JS `\w`
/// is ASCII-only).
fn is_runes_token_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'$' | b'-')
}

/// Iterate the `svelte-ignore` tail the way upstream's runes-mode
/// regex `/([\w$-]+)(,)?/gm` does: yields `(byte_offset, token,
/// followed_by_comma)` for each token run, where `followed_by_comma`
/// is true only when a `,` IMMEDIATELY follows the token (no
/// whitespace between). Callers stop consuming after the first token
/// yielded with `followed_by_comma == false` — everything after it is
/// prose.
fn runes_ignore_tokens(rest: &str) -> impl Iterator<Item = (usize, &str, bool)> {
    let bytes = rest.as_bytes();
    let mut i = 0usize;
    std::iter::from_fn(move || {
        while i < bytes.len() && !is_runes_token_byte(bytes[i]) {
            i += 1;
        }
        if i >= bytes.len() {
            return None;
        }
        let start = i;
        while i < bytes.len() && is_runes_token_byte(bytes[i]) {
            i += 1;
        }
        let followed_by_comma = bytes.get(i) == Some(&b',');
        Some((start, &rest[start..i], followed_by_comma))
    })
}

/// Closest known code name to `input` via Levenshtein similarity.
/// Returns `None` when nothing clears the 0.7 threshold (mirrors
/// upstream fuzzymatch.js).
fn fuzzymatch_known_code(input: &str) -> Option<&'static str> {
    let target = input.to_ascii_lowercase();
    let mut best: Option<(f64, &'static str)> = None;
    let candidates = crate::codes::CODES
        .iter()
        .copied()
        .chain(IGNORABLE_RUNTIME_WARNINGS.iter().copied());
    for c in candidates {
        let sim = lev_similarity(&target, c);
        // Upstream's fuzzymatch.js:12 gates on `> 0.7` (strict), so a
        // typo scoring exactly 0.7 produces no suggestion.
        if sim > 0.7 && best.map(|(s, _)| sim > s).unwrap_or(true) {
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
    Code::try_from_str(code).is_some() || IGNORABLE_RUNTIME_WARNINGS.contains(&code)
}

fn legacy_rename(code: &str) -> Option<&'static str> {
    LEGACY_RENAMES
        .iter()
        .find_map(|(from, to)| if *from == code { Some(*to) } else { None })
}
