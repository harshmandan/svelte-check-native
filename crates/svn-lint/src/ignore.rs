//! `<!-- svelte-ignore CODE [, CODE, …] -->` extractor.
//!
//! Mirrors `svelte/src/compiler/utils/extract_svelte_ignore.js` —
//! including the 9 legacy→new rename entries + runes-mode comma
//! parsing + lax non-runes-mode parsing.

use oxc_ast::AstKind;
use oxc_ast::ast::{ArrayExpressionElement, Program, TemplateLiteral};
use oxc_ast_visit::Visit;
use oxc_span::{GetSpan, Span};
use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::ast::{Comment, Node};

use crate::codes::{COMPILER_ERROR_CODES, Code};
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
    nodes: &[&Node],
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
        match nodes[i] {
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

/// Does this comment's `svelte-ignore` list name `code`? Parsing the
/// comment reports its `legacy_code` / `unknown_code` tokens again on
/// every call, exactly as each upstream `extract_svelte_ignore` call
/// does.
pub fn comment_ignores_code(c: &Comment, code: Code, ctx: &mut LintContext<'_>) -> bool {
    extract_from_comment(c, ctx)
        .iter()
        .any(|ignored| ignored.as_str() == code.as_str())
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

/// The compiler's `/[\w$-]/` — ASCII only, so a multi-byte character
/// ends a code rather than joining it.
fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'$')
}

/// The `svelte-ignore` comments of one script body, keyed by the node
/// each one leads.
///
/// The compiler parses a script with acorn and attaches comments to
/// the estree afterwards (`phases/1-parse/acorn.js`, `add_comments`):
/// walking the tree, every queued comment that starts before a node
/// becomes a `leadingComment` of that node, and after a node's children
/// the next comment becomes its `trailingComment` when it is the last
/// node of a block, array or object, or when only `,`, `)`, spaces and
/// tabs separate the two. The analyze walk then applies the ignores of
/// a node's leading comments to the node and everything inside it
/// (`2-analyze/index.js`). [`ScriptComments::build`] replays that
/// attachment over the oxc tree.
pub(crate) struct ScriptComments {
    /// `(node start, codes)` for every `svelte-ignore` comment attached
    /// as a leading comment, in comment order. Offsets are
    /// script-local, matching oxc spans.
    leading: Vec<(u32, Vec<SmolStr>)>,
}

impl ScriptComments {
    /// No comments at all — every lookup answers "no codes".
    pub(crate) fn empty() -> Self {
        Self {
            leading: Vec::new(),
        }
    }

    /// Attach the comments of `program` (parsed from `content`) the way
    /// acorn's comment handler does and keep the `svelte-ignore` codes
    /// of every leading comment.
    pub(crate) fn build(program: &Program<'_>, content: &str, runes: bool) -> Self {
        let codes: Vec<Vec<SmolStr>> = program
            .comments
            .iter()
            .map(|c| {
                content
                    .get(c.span.start as usize..c.span.end as usize)
                    .and_then(crate::scope_util::strip_comment_delimiters)
                    .and_then(|body| {
                        let rest = body.trim_start().strip_prefix("svelte-ignore")?;
                        match rest.chars().next() {
                            Some(ch) if ch.is_whitespace() => {
                                Some(parse_ignore_codes(&rest[ch.len_utf8()..], runes))
                            }
                            _ => None,
                        }
                    })
                    .unwrap_or_default()
            })
            .collect();
        if codes.iter().all(Vec::is_empty) {
            return Self::empty();
        }
        let mut queue: Vec<(u32, u32, usize)> = program
            .comments
            .iter()
            .enumerate()
            .map(|(i, c)| (c.span.start, c.span.end, i))
            .collect();
        queue.sort_by_key(|&(start, _, _)| start);
        let mut attacher = CommentAttacher {
            source: content,
            queue: queue.into(),
            frames: Vec::new(),
            skipped: Vec::new(),
            leading: Vec::new(),
        };
        attacher.visit_program(program);
        let leading = attacher
            .leading
            .into_iter()
            .filter_map(|(start, i)| {
                let c = codes.get(i).filter(|c| !c.is_empty())?;
                Some((start, c.clone()))
            })
            .collect();
        Self { leading }
    }

    pub(crate) fn has_ignores(&self) -> bool {
        !self.leading.is_empty()
    }

    /// Suppression codes of the comments leading the node that starts
    /// at `node_start` (script-local offset). A node shares its start
    /// with the nodes it begins with; the compiler attaches to the
    /// outermost of them, whose subtree holds the others.
    pub(crate) fn leading_ignores(&self, node_start: u32) -> Vec<SmolStr> {
        let mut codes: Vec<SmolStr> = Vec::new();
        for (_, entry) in self.leading.iter().filter(|(s, _)| *s == node_start) {
            for c in entry {
                if !codes.contains(c) {
                    codes.push(c.clone());
                }
            }
        }
        codes
    }
}

/// Replays acorn.js's `add_comments` over an oxc tree. The estree acorn
/// builds differs from oxc's in a few nodes, which are passed over:
/// parentheses (scripts are parsed without `preserveParens`), array
/// holes (`null` in estree) and the parameter list wrapper.
struct CommentAttacher<'s> {
    source: &'s str,
    /// Comments not yet attached, by start: `(start, end, index)`.
    queue: std::collections::VecDeque<(u32, u32, usize)>,
    /// The estree nodes currently open, outermost first.
    frames: Vec<Frame>,
    /// One entry per entered oxc node: whether it was passed over.
    skipped: Vec<bool>,
    /// `(node start, comment index)` of every leading comment.
    leading: Vec<(u32, usize)>,
}

struct Frame {
    span: Span,
    /// The span of the node's last child in a list whose last node
    /// keeps every comment up to the list's end (a block or program
    /// body, array elements, object properties).
    last_in_list: Option<Span>,
}

fn passed_over(kind: AstKind<'_>) -> bool {
    matches!(
        kind,
        AstKind::ParenthesizedExpression(_)
            | AstKind::Elision(_)
            | AstKind::FormalParameters(_)
            | AstKind::Hashbang(_)
    )
}

fn last_in_list(kind: AstKind<'_>) -> Option<Span> {
    match kind {
        AstKind::Program(p) => p
            .body
            .last()
            .map(GetSpan::span)
            .or_else(|| p.directives.last().map(|d| d.span)),
        AstKind::BlockStatement(b) => b.body.last().map(GetSpan::span),
        AstKind::FunctionBody(b) => b
            .statements
            .last()
            .map(GetSpan::span)
            .or_else(|| b.directives.last().map(|d| d.span)),
        AstKind::ArrayExpression(a) => a.elements.last().and_then(|el| match el {
            ArrayExpressionElement::Elision(_) => None,
            ArrayExpressionElement::SpreadElement(s) => Some(s.span),
            other => other
                .as_expression()
                .map(|e| e.without_parentheses().span()),
        }),
        AstKind::ObjectExpression(o) => o.properties.last().map(GetSpan::span),
        _ => None,
    }
}

impl<'a> Visit<'a> for CommentAttacher<'_> {
    /// Acorn creates a template literal's `expressions` before its
    /// `quasis`, and the estree walk visits keys in creation order.
    fn visit_template_literal(&mut self, it: &TemplateLiteral<'a>) {
        let kind = AstKind::TemplateLiteral(self.alloc(it));
        self.enter_node(kind);
        self.visit_expressions(&it.expressions);
        self.visit_template_elements(&it.quasis);
        self.leave_node(kind);
    }

    fn enter_node(&mut self, kind: AstKind<'a>) {
        let skip = passed_over(kind);
        self.skipped.push(skip);
        if skip {
            return;
        }
        let span = kind.span();
        while let Some(&(start, _, index)) = self.queue.front()
            && start < span.start
        {
            self.queue.pop_front();
            self.leading.push((span.start, index));
        }
        self.frames.push(Frame {
            span,
            last_in_list: last_in_list(kind),
        });
    }

    fn leave_node(&mut self, _kind: AstKind<'a>) {
        if self.skipped.pop() != Some(false) {
            return;
        }
        let Some(node) = self.frames.pop() else {
            return;
        };
        let Some(&(next_start, _, _)) = self.queue.front() else {
            return;
        };
        let parent = self.frames.last();
        if parent.is_some_and(|p| p.span.end == node.span.end) {
            return;
        }
        if let Some(parent) = parent.filter(|p| p.last_in_list == Some(node.span)) {
            while self
                .queue
                .front()
                .is_some_and(|&(start, _, _)| start < parent.span.end)
            {
                self.queue.pop_front();
            }
        } else if node.span.end <= next_start
            && self
                .source
                .get(node.span.end as usize..next_start as usize)
                .is_some_and(|gap| gap.bytes().all(|b| matches!(b, b',' | b')' | b' ' | b'\t')))
        {
            self.queue.pop_front();
        }
    }
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
        .filter(|c| !COMPILER_ERROR_CODES.contains(c))
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

/// A code a `svelte-ignore` comment can name: the compiler's warning
/// codes plus the ignorable runtime warnings. Our catalog also lists
/// the compiler errors the lint pass reports; those are not codes to
/// the compiler's ignore parser.
fn is_known_code(code: &str) -> bool {
    (Code::try_from_str(code).is_some() && !COMPILER_ERROR_CODES.contains(&code))
        || IGNORABLE_RUNTIME_WARNINGS.contains(&code)
}

fn legacy_rename(code: &str) -> Option<&'static str> {
    LEGACY_RENAMES
        .iter()
        .find_map(|(from, to)| if *from == code { Some(*to) } else { None })
}
