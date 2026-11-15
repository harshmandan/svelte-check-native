//! `<!-- svelte-ignore CODE [, CODE, …] -->` extractor.
//!
//! Mirrors `svelte/src/compiler/utils/extract_svelte_ignore.js` —
//! including the 9 legacy→new rename entries + runes-mode comma
//! parsing + lax non-runes-mode parsing.

use oxc_ast::AstKind;
use oxc_ast::ast::{ArrayExpressionElement, ArrowFunctionExpression, Program, TemplateLiteral};
use oxc_ast_visit::{Visit, walk};
use oxc_span::{GetSpan, Span};
use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::ast::{Comment, Fragment, Node};

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
/// non-Comment / non-Text sibling (`<svelte:options>` is none: the
/// parser lifts it out of the template). Mirrors the upstream analyze
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
            // Not a sibling: the parser takes it out of the template.
            node if is_lifted_options(node) => {}
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
    let Some(rest) = ignore_list(data) else {
        return Vec::new();
    };
    // Where the code list begins in the source: the comment body starts
    // after the `<!--` delimiter.
    let tokens_start = c.data_range.start + (data.len() - rest.len()) as u32;
    let runes = ctx.runes;
    ignore_codes(rest, runes, |start, token| {
        let range = Range::new(
            tokens_start + start as u32,
            tokens_start + (start + token.len()) as u32,
        );
        let (code, message) = bad_code_warning(token);
        ctx.emit(code, message, range);
    })
}

/// The code list of a comment whose text is `data`: what follows the
/// compiler's `/^\s*svelte-ignore\s/`, or `None` when it doesn't match.
pub(crate) fn ignore_list(data: &str) -> Option<&str> {
    let rest = data
        .trim_start_matches(svn_parser::is_svelte_whitespace)
        .strip_prefix("svelte-ignore")?;
    let ws = rest
        .chars()
        .next()
        .filter(|c| svn_parser::is_svelte_whitespace(*c))?;
    Some(&rest[ws.len_utf8()..])
}

/// The warning the compiler raises for a `svelte-ignore` token that
/// names no warning: `legacy_code` when its rename does, else
/// `unknown_code` with the closest code as a suggestion.
fn bad_code_warning(token: &str) -> (Code, String) {
    let replacement = legacy_rename(token)
        .map(str::to_string)
        .unwrap_or_else(|| token.replace('-', "_"));
    if is_known_code(&replacement) {
        (
            Code::legacy_code,
            messages::legacy_code(token, &replacement),
        )
    } else {
        (
            Code::unknown_code,
            messages::unknown_code(token, fuzzymatch_known_code(token)),
        )
    }
}

/// The suppression codes of a `svelte-ignore` code list (what follows
/// `svelte-ignore `), as `extract_svelte_ignore` reads them. In runes
/// mode the list is comma-separated and ends at the first token not
/// immediately followed by a comma; a token that names no warning is
/// passed to `bad` (with its offset in `rest`) and suppresses nothing.
/// In legacy mode every word-ish token is a code, and a renamed code
/// suppresses under both names.
fn ignore_codes(rest: &str, runes: bool, mut bad: impl FnMut(usize, &str)) -> Vec<SmolStr> {
    let mut out: Vec<SmolStr> = Vec::new();
    let mut push = |code: &str| {
        if !out.iter().any(|c| c == code) {
            out.push(SmolStr::new(code));
        }
    };
    if runes {
        for (start, token, followed_by_comma) in runes_ignore_tokens(rest) {
            if is_known_code(token) {
                push(token);
            } else {
                bad(start, token);
            }
            if !followed_by_comma {
                break;
            }
        }
    } else {
        for (_, token, _) in runes_ignore_tokens(rest) {
            push(token);
            if !is_known_code(token) {
                let mapped = legacy_rename(token)
                    .map(str::to_string)
                    .unwrap_or_else(|| token.replace('-', "_"));
                if is_known_code(&mapped) {
                    push(&mapped);
                }
            }
        }
    }
    out
}

/// Parse the tail of `svelte-ignore ` into the list of suppression
/// codes, reporting nothing (see [`ignore_codes`]).
pub fn parse_ignore_codes_public(rest: &str, runes: bool) -> Vec<SmolStr> {
    ignore_codes(rest, runes, |_, _| {})
}

/// Whether reading this code list reports a token that names no
/// warning (runes mode only).
pub(crate) fn ignore_list_has_bad_code(rest: &str, runes: bool) -> bool {
    let mut found = false;
    ignore_codes(rest, runes, |_, _| found = true);
    found
}

/// The `svelte-ignore` comments of one script body or template
/// expression, keyed by the node each one leads.
///
/// The compiler parses a script with acorn and attaches comments to
/// the estree afterwards (`phases/1-parse/acorn.js`, `add_comments`):
/// walking the tree, every queued comment that starts before a node
/// becomes a `leadingComment` of that node, and after a node's children
/// the next comment becomes its `trailingComment` when it is the last
/// node of a block, array or object, or when only `,`, `)`, spaces and
/// tabs separate the two. So a comment leads exactly one node: the
/// outermost one starting after it. The analysis walk visits every
/// node and, at each, reads its leading comments — reporting the codes
/// that name no warning then, once — and applies their ignores to the
/// node and everything inside it (`2-analyze/index.js`).
/// [`ScriptComments::build`] replays that attachment over the oxc tree.
///
/// Our walk does not stop at every node, so a node's ignores apply at
/// every node it visits inside the leading node's span, and its codes
/// are reported at the first node visited at or after its start.
pub(crate) struct ScriptComments {
    /// Every node with a leading `svelte-ignore` comment, by start;
    /// spans are offsets into the walked text, matching oxc spans.
    leading: Vec<LeadingIgnores>,
    /// The first entry of `leading` whose bad codes are not reported
    /// yet.
    next_report: usize,
}

struct LeadingIgnores {
    start: u32,
    end: u32,
    codes: Vec<SmolStr>,
    /// The codes that name no warning, in the comment's order.
    bad: Vec<BadIgnoreCode>,
}

/// Where the text a comment set was parsed from sits in the file.
pub(crate) struct CommentSource<'s> {
    /// The whole file.
    pub(crate) file: &'s str,
    /// Where the parsed text's offset 0 lies in the file.
    pub(crate) base: u32,
    /// Everything before this file offset was blanked to spaces in
    /// what the compiler parsed (a script is parsed with the markup
    /// before it blanked, newlines kept); `0` for a template
    /// expression, read in place.
    pub(crate) blank_before: u32,
}

/// How the tree the compiler's analysis walks differs from the one oxc
/// parses from the same text: what the parser did after attaching the
/// comments. The default is a script, read as parsed.
#[derive(Default)]
pub(crate) struct CompilerRead<'r> {
    /// Comments queued ahead of the text's own when they were attached,
    /// in queue order and file offsets (see
    /// `EachBlock::failed_read_comments`).
    pub(crate) queued_first: &'r [Range],
    /// The text is a template expression: read as an expression, with
    /// no statement around it, and with its parentheses as nodes,
    /// removed after the comments were attached (acorn's
    /// `preserveParens`, then `remove_parens`), so a comment leading a
    /// `(…)` leads nothing.
    pub(crate) template_expression: bool,
    /// Nodes the compiler dropped for an inner one (see
    /// `EachBlock::dropped_nodes`), in file offsets.
    pub(crate) dropped_nodes: &'r [Range],
    /// The analysis visits only the nodes inside these ranges (file
    /// offsets); `None` visits every node.
    pub(crate) visited: Option<&'r [Range]>,
}

impl ScriptComments {
    /// No comments at all — every lookup answers "no codes".
    pub(crate) fn empty() -> Self {
        Self {
            leading: Vec::new(),
            next_report: 0,
        }
    }

    /// Attach the comments of `program` the way acorn's comment
    /// handler does and keep the `svelte-ignore` codes of every
    /// leading comment.
    ///
    /// `prior` holds the codes of the comments the compiler read before
    /// this script (see [`prior_script_ignores`]); they lead the
    /// script's first node. `read` says how the compiler's tree differs
    /// from the one `program` shows.
    pub(crate) fn build(
        program: &Program<'_>,
        from: &CommentSource<'_>,
        runes: bool,
        prior: PriorIgnores,
        read: &CompilerRead<'_>,
    ) -> Self {
        let file = from.file;
        let codes: Vec<(Vec<SmolStr>, Vec<BadIgnoreCode>)> = program
            .comments
            .iter()
            .map(|c| {
                let range = Range::new(from.base + c.span.start, from.base + c.span.end);
                let text = range.slice(file);
                if !text.contains("svelte-ignore") {
                    return (Vec::new(), Vec::new());
                }
                (
                    comment_ignore_codes(text, runes),
                    comment_bad_codes(
                        file,
                        text,
                        range.start,
                        runes,
                        &comment_indentation(file, range.start, from.blank_before),
                    ),
                )
            })
            .collect();
        let nothing =
            |(codes, bad): &(Vec<SmolStr>, Vec<BadIgnoreCode>)| codes.is_empty() && bad.is_empty();
        if prior.codes.is_empty() && prior.bad_codes.is_empty() && codes.iter().all(nothing) {
            return Self::empty();
        }
        let content = file.get(from.base as usize..).unwrap_or_default();
        let mut own: Vec<(u32, u32, usize)> = program
            .comments
            .iter()
            .enumerate()
            .map(|(i, c)| (c.span.start, c.span.end, i))
            .collect();
        own.sort_by_key(|&(start, _, _)| start);
        // A comment queued ahead is one of the text's own when it lies
        // inside the text; one that does not has no codes here.
        let ahead = read.queued_first.iter().filter_map(|range| {
            let start = range.start.checked_sub(from.base)?;
            let index = own
                .iter()
                .find(|&&(own_start, _, _)| own_start == start)
                .map_or(usize::MAX, |&(_, _, i)| i);
            Some((start, range.end - from.base, index))
        });
        let queue: std::collections::VecDeque<_> = ahead.chain(own.iter().copied()).collect();
        let mut attacher = CommentAttacher {
            source: content,
            template_expression: read.template_expression,
            queue,
            frames: Vec::new(),
            skipped: Vec::new(),
            leading: Vec::new(),
            first_node: None,
        };
        attacher.visit_program(program);
        let mut leading: Vec<LeadingIgnores> = Vec::new();
        if let Some(first) = attacher.first_node
            && !(prior.codes.is_empty() && prior.bad_codes.is_empty())
        {
            leading.push(LeadingIgnores {
                start: first.start,
                end: first.end,
                codes: prior.codes,
                bad: prior.bad_codes,
            });
        }
        // Whether the compiler, by the time its analysis walks the tree,
        // has thrown away the node the comment leads or never visits it.
        let leads_nothing = |led: &Led| {
            let node = Range::new(from.base + led.node.start, from.base + led.node.end);
            let comment_start = from.base + led.comment_start;
            led.removed_parens
                || read.dropped_nodes.iter().any(|dropped| {
                    comment_start < dropped.start
                        && (dropped.start..=dropped.end).contains(&node.start)
                })
                || read.visited.is_some_and(|visited| {
                    !visited
                        .iter()
                        .any(|v| v.start <= node.start && node.end <= v.end)
                })
        };
        leading.extend(attacher.leading.into_iter().filter_map(|led| {
            if leads_nothing(&led) {
                return None;
            }
            let (span, i) = (led.node, led.comment);
            let (codes, bad) = codes.get(i).filter(|c| !nothing(c))?;
            Some(LeadingIgnores {
                start: span.start,
                end: span.end,
                codes: codes.clone(),
                bad: bad.clone(),
            })
        }));
        leading.sort_by_key(|entry| entry.start);
        Self {
            leading,
            next_report: 0,
        }
    }

    pub(crate) fn has_ignores(&self) -> bool {
        !self.leading.is_empty()
    }

    /// The walk enters a node spanning `start..end`. Returns the codes
    /// naming no warning of the nodes starting at or before it that
    /// are not reported yet, and the suppression codes of every node
    /// whose span holds this one.
    pub(crate) fn enter(&mut self, start: u32, end: u32) -> (Vec<BadIgnoreCode>, Vec<SmolStr>) {
        let mut bad = Vec::new();
        while let Some(entry) = self.leading.get(self.next_report)
            && entry.start <= start
        {
            bad.extend(entry.bad.iter().cloned());
            self.next_report += 1;
        }
        let mut codes: Vec<SmolStr> = Vec::new();
        for entry in &self.leading {
            if entry.start > start {
                break;
            }
            if end <= entry.end {
                for c in &entry.codes {
                    if !codes.contains(c) {
                        codes.push(c.clone());
                    }
                }
            }
        }
        (bad, codes)
    }

    /// The codes naming no warning of the nodes the walk never reached.
    pub(crate) fn take_unreported(&mut self) -> Vec<BadIgnoreCode> {
        let rest = self.leading.get(self.next_report..).unwrap_or_default();
        let bad = rest.iter().flat_map(|e| e.bad.iter().cloned()).collect();
        self.next_report = self.leading.len();
        bad
    }
}

/// A code in a `svelte-ignore` comment that names no warning: the
/// `legacy_code` or `unknown_code` warning the compiler raises for it,
/// at the code.
#[derive(Clone)]
pub(crate) struct BadIgnoreCode {
    pub(crate) code: Code,
    pub(crate) message: String,
    pub(crate) range: Range,
}

/// What a script walk is told about the comments the compiler read
/// before the script: their `svelte-ignore` codes, and the codes among
/// them that name no warning.
#[derive(Default)]
pub(crate) struct PriorIgnores {
    pub(crate) codes: Vec<SmolStr>,
    pub(crate) bad_codes: Vec<BadIgnoreCode>,
}

/// The codes that name no warning in a JavaScript `svelte-ignore`
/// comment, whose text (delimiters included) starts at `start` in the
/// file. The compiler only checks the codes in runes mode.
///
/// The compiler reads the comment's value as acorn's comment hook left
/// it: a block comment spanning lines loses `indentation` — the spaces
/// and tabs opening the line the comment starts on — from the start of
/// every line (the first included), and the positions it reports are
/// offsets into that shortened value, counted from just past the `/*`.
fn comment_bad_codes(
    file: &str,
    text: &str,
    start: u32,
    runes: bool,
    indentation: &str,
) -> Vec<BadIgnoreCode> {
    if !runes {
        return Vec::new();
    }
    let Some(value) = crate::scope_util::strip_comment_delimiters(text) else {
        return Vec::new();
    };
    if text.starts_with("/*") && value.contains('\n') {
        let value = strip_indentation(value, indentation);
        bad_codes_in(file, &value, start + 2)
    } else {
        bad_codes_in(file, value, start + 2)
    }
}

/// The codes that name no warning in the `svelte-ignore` list of
/// `value`, a comment's text, as the compiler's `extract_svelte_ignore`
/// reports them in runes mode: at the position it is told the value
/// starts at, `value_start` in `file`, plus the token's offset in
/// `value`. That offset counts UTF-16 units of `value` — which may be
/// shorter than the text in the file — so the position is found by
/// counting the same number of units on through `file`.
fn bad_codes_in(file: &str, value: &str, value_start: u32) -> Vec<BadIgnoreCode> {
    let Some(rest) = ignore_list(value) else {
        return Vec::new();
    };
    let rest_at = value.len() - rest.len();
    let mut out = Vec::new();
    ignore_codes(rest, true, |at, token| {
        let offset = value[..rest_at + at].encode_utf16().count();
        let start = advance_utf16(file, value_start, offset);
        let range = Range::new(start, advance_utf16(file, start, token.len()));
        let (code, message) = bad_code_warning(token);
        out.push(BadIgnoreCode {
            code,
            message,
            range,
        });
    });
    out
}

/// The byte offset `units` UTF-16 code units on from byte offset `from`
/// in `file`, stopping at the end of the file.
fn advance_utf16(file: &str, from: u32, units: usize) -> u32 {
    let mut left = units;
    let mut at = from as usize;
    for c in file.get(at..).unwrap_or_default().chars() {
        if left == 0 {
            break;
        }
        left = left.saturating_sub(c.len_utf16());
        at += c.len_utf8();
    }
    at as u32
}

/// The spaces and tabs opening the line of `file` that a comment
/// starting at `comment_start` sits on, as the text the compiler parsed
/// has them: everything before `blank_before` is a space there (one per
/// UTF-16 unit, newlines kept).
fn comment_indentation(file: &str, comment_start: u32, blank_before: u32) -> String {
    let head = file.get(..comment_start as usize).unwrap_or_default();
    let line_start = head.rfind('\n').map_or(0, |i| i + 1);
    let mut indentation = String::new();
    let mut at = line_start;
    let blank_end = (blank_before as usize).min(comment_start as usize);
    if at < blank_end {
        let blanked = file.get(at..blank_end).unwrap_or_default();
        indentation.extend(std::iter::repeat_n(' ', blanked.encode_utf16().count()));
        at = blank_end;
    }
    let rest = file.get(at..).unwrap_or_default();
    let run = rest
        .bytes()
        .take_while(|b| matches!(b, b' ' | b'\t'))
        .count();
    indentation.push_str(&rest[..run]);
    indentation
}

/// `value` with `indentation` removed from the start of each line, the
/// first included — JavaScript's `value.replace(/^<indentation>/gm, '')`,
/// whose lines end at `\n`, `\r`, U+2028 and U+2029.
fn strip_indentation(value: &str, indentation: &str) -> String {
    if indentation.is_empty() {
        return value.to_string();
    }
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    loop {
        if let Some(stripped) = rest.strip_prefix(indentation) {
            rest = stripped;
        }
        match rest.find(['\n', '\r', '\u{2028}', '\u{2029}']) {
            Some(i) => {
                let break_len = rest[i..].chars().next().map_or(1, char::len_utf8);
                out.push_str(&rest[..i + break_len]);
                rest = &rest[i + break_len..];
            }
            None => {
                out.push_str(rest);
                return out;
            }
        }
    }
}

/// The `legacy_code` / `unknown_code` warnings for the `svelte-ignore`
/// comment right before `<style>`. After the template, the compiler
/// reads that comment once more to see whether it ignores
/// `css_unused_selector` — giving the comment's start as where its text
/// starts, without the `<!--` — and warns about its codes again.
pub(crate) fn style_comment_bad_codes(
    fragment: &Fragment,
    style_start: u32,
    ctx: &mut LintContext<'_>,
) {
    if !ctx.runes {
        return;
    }
    let Some(comment) = comment_before_tag(fragment, style_start, ctx.source) else {
        return;
    };
    let data = comment.data_range.slice(ctx.source);
    for bad in bad_codes_in(ctx.source, data, comment.range.start) {
        ctx.emit(bad.code, bad.message, bad.range);
    }
}

/// The comment the compiler's parser keeps for a top-level `<script>`
/// or `<style>` whose tag opens at `tag_start` (`element.js`): the node
/// right before the tag must end where it opens, and walking back over
/// text that is white space once decoded, the first comment met. Any
/// other node — `<svelte:options>` included, which is still in the
/// template while the parser reads the tag — ends the walk.
pub(crate) fn comment_before_tag<'f>(
    fragment: &'f Fragment,
    tag_start: u32,
    source: &str,
) -> Option<&'f Comment> {
    let mut before = fragment
        .nodes
        .iter()
        .rev()
        .skip_while(|node| node.range().end > tag_start)
        .peekable();
    if before.peek()?.range().end != tag_start {
        return None;
    }
    for node in before {
        match node {
            Node::Comment(c) => return Some(c),
            Node::Text(t)
                if crate::html_decode::decode_character_references(
                    t.range.slice(source),
                    false,
                )
                .trim_matches(svn_parser::is_svelte_whitespace)
                .is_empty() => {}
            _ => return None,
        }
    }
    None
}

/// `<svelte:options>`, which the parser takes out of the template: it is
/// no sibling of the nodes around it.
pub(crate) fn is_lifted_options(node: &Node) -> bool {
    matches!(node, Node::SvelteElement(se) if se.kind == svn_parser::ast::SvelteElementKind::Options)
}

/// The `svelte-ignore` codes a JavaScript comment's text (delimiters
/// included) carries; empty for any other comment.
fn comment_ignore_codes(text: &str, runes: bool) -> Vec<SmolStr> {
    crate::scope_util::strip_comment_delimiters(text)
        .and_then(ignore_list)
        .map(|rest| parse_ignore_codes_public(rest, runes))
        .unwrap_or_default()
}

/// The `svelte-ignore` codes of every JavaScript comment the compiler
/// has read when it reaches a script whose body starts at `before`.
///
/// The compiler's parser keeps one list of the JavaScript comments it
/// meets — in template expressions, in start tags and in script bodies
/// — and parses each `<script>` with the whole list
/// (`phases/1-parse/read/script.js`), so every comment read earlier is
/// attached as a leading comment of the script's first node. HTML
/// comments, text and `<style>` are not JavaScript and add nothing.
/// `scripts` are the other script bodies as `(content start, program)`.
pub(crate) fn prior_script_ignores(
    fragment: Option<&Fragment>,
    source: &str,
    before: u32,
    scripts: &[(u32, &Program<'_>)],
    runes: bool,
) -> PriorIgnores {
    // Cheap gate: no ignore comment can precede the script.
    if !source
        .get(..before as usize)
        .is_some_and(|head| head.contains("svelte-ignore"))
    {
        return PriorIgnores::default();
    }
    // Each comment with where the text it was lexed from stops being
    // blanked (see `CommentSource::blank_before`).
    let mut comments: Vec<(Range, u32)> = Vec::new();
    for &(content_start, program) in scripts {
        if content_start < before {
            comments.extend(program.comments.iter().map(|c| {
                (
                    Range::new(content_start + c.span.start, content_start + c.span.end),
                    content_start,
                )
            }));
        }
    }
    if let Some(fragment) = fragment {
        comments.extend(
            fragment
                .comments
                .iter()
                .filter(|c| c.start < before)
                .map(|&range| (range, 0)),
        );
    }
    comments.sort_by_key(|(range, _)| range.start);
    let mut prior = PriorIgnores::default();
    for (range, blank_before) in comments {
        for code in comment_ignore_codes(range.slice(source), runes) {
            if !prior.codes.contains(&code) {
                prior.codes.push(code);
            }
        }
        // Read again as a leading comment of this script's first node,
        // a code that names no warning is warned about again.
        prior.bad_codes.extend(comment_bad_codes(
            source,
            range.slice(source),
            range.start,
            runes,
            &comment_indentation(source, range.start, blank_before),
        ));
    }
    prior
}

/// Replays acorn.js's `add_comments` over an oxc tree. The estree acorn
/// builds differs from oxc's in a few nodes, which are passed over:
/// parentheses (scripts are parsed without `preserveParens`), array
/// holes (`null` in estree) and the parameter list wrapper.
struct CommentAttacher<'s> {
    source: &'s str,
    /// See [`CompilerRead::template_expression`].
    template_expression: bool,
    /// Comments not yet attached, in queue order: `(start, end, index)`.
    queue: std::collections::VecDeque<(u32, u32, usize)>,
    /// The estree nodes currently open, outermost first.
    frames: Vec<Frame>,
    /// One entry per entered oxc node: whether it was passed over.
    skipped: Vec<bool>,
    /// Every leading comment.
    leading: Vec<Led>,
    /// The first node inside the program.
    first_node: Option<Span>,
}

/// A comment leading a node.
struct Led {
    node: Span,
    comment: usize,
    comment_start: u32,
    /// The node is parentheses the compiler removes.
    removed_parens: bool,
}

struct Frame {
    span: Span,
    /// The span of the node's last child in a list whose last node
    /// keeps every comment up to the list's end (a block or program
    /// body, array elements, object properties).
    last_in_list: Option<Span>,
}

fn passed_over(kind: AstKind<'_>, parens_are_nodes: bool) -> bool {
    match kind {
        AstKind::ParenthesizedExpression(_) => !parens_are_nodes,
        AstKind::Elision(_) | AstKind::FormalParameters(_) | AstKind::Hashbang(_) => true,
        _ => false,
    }
}

fn last_in_list(kind: AstKind<'_>, parens_are_nodes: bool) -> Option<Span> {
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
            other => other.as_expression().map(|e| {
                if parens_are_nodes {
                    e.span()
                } else {
                    e.without_parentheses().span()
                }
            }),
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

    /// acorn-typescript gives a non-async arrow function its
    /// `returnType` first and its `typeParameters` last, so the walk
    /// reaches the parameters before a comment among the type
    /// parameters can lead anything there. An async arrow keeps oxc's
    /// order.
    fn visit_arrow_function_expression(&mut self, it: &ArrowFunctionExpression<'a>) {
        if it.r#async {
            walk::walk_arrow_function_expression(self, it);
            return;
        }
        let kind = AstKind::ArrowFunctionExpression(self.alloc(it));
        self.enter_node(kind);
        if let Some(return_type) = &it.return_type {
            self.visit_ts_type_annotation(return_type);
        }
        self.visit_formal_parameters(&it.params);
        self.visit_arrow_function_body(&it.body);
        if let Some(type_parameters) = &it.type_parameters {
            self.visit_ts_type_parameter_declaration(type_parameters);
        }
        self.leave_node(kind);
    }

    fn enter_node(&mut self, kind: AstKind<'a>) {
        // A template expression is parsed as a statement here; the
        // compiler's root is the expression itself.
        let skip = passed_over(kind, self.template_expression)
            || (self.template_expression
                && matches!(kind, AstKind::ExpressionStatement(_))
                && self.frames.len() == 1);
        self.skipped.push(skip);
        if skip {
            return;
        }
        let span = kind.span();
        if self.first_node.is_none() && !matches!(kind, AstKind::Program(_)) {
            self.first_node = Some(span);
        }
        while let Some(&(start, _, index)) = self.queue.front()
            && start < span.start
        {
            self.queue.pop_front();
            self.leading.push(Led {
                node: span,
                comment: index,
                comment_start: start,
                removed_parens: matches!(kind, AstKind::ParenthesizedExpression(_)),
            });
        }
        self.frames.push(Frame {
            span,
            last_in_list: last_in_list(kind, self.template_expression),
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

/// Every code a `svelte-ignore` comment can name, in the compiler's
/// order: its warning codes (`warnings.js` `codes`) followed by
/// `IGNORABLE_RUNTIME_WARNINGS`. The order decides between equally
/// close suggestions.
const IGNORE_CODES: &[&str] = &[
    "a11y_accesskey",
    "a11y_aria_activedescendant_has_tabindex",
    "a11y_aria_attributes",
    "a11y_autocomplete_valid",
    "a11y_autofocus",
    "a11y_click_events_have_key_events",
    "a11y_consider_explicit_label",
    "a11y_distracting_elements",
    "a11y_figcaption_index",
    "a11y_figcaption_parent",
    "a11y_hidden",
    "a11y_img_redundant_alt",
    "a11y_incorrect_aria_attribute_type",
    "a11y_incorrect_aria_attribute_type_boolean",
    "a11y_incorrect_aria_attribute_type_id",
    "a11y_incorrect_aria_attribute_type_idlist",
    "a11y_incorrect_aria_attribute_type_integer",
    "a11y_incorrect_aria_attribute_type_token",
    "a11y_incorrect_aria_attribute_type_tokenlist",
    "a11y_incorrect_aria_attribute_type_tristate",
    "a11y_interactive_supports_focus",
    "a11y_invalid_attribute",
    "a11y_label_has_associated_control",
    "a11y_media_has_caption",
    "a11y_misplaced_role",
    "a11y_misplaced_scope",
    "a11y_missing_attribute",
    "a11y_missing_content",
    "a11y_mouse_events_have_key_events",
    "a11y_no_abstract_role",
    "a11y_no_interactive_element_to_noninteractive_role",
    "a11y_no_noninteractive_element_interactions",
    "a11y_no_noninteractive_element_to_interactive_role",
    "a11y_no_noninteractive_tabindex",
    "a11y_no_redundant_roles",
    "a11y_no_static_element_interactions",
    "a11y_positive_tabindex",
    "a11y_role_has_required_aria_props",
    "a11y_role_supports_aria_props",
    "a11y_role_supports_aria_props_implicit",
    "a11y_unknown_aria_attribute",
    "a11y_unknown_role",
    "bidirectional_control_characters",
    "legacy_code",
    "unknown_code",
    "options_deprecated_accessors",
    "options_deprecated_immutable",
    "options_missing_custom_element",
    "options_removed_enable_sourcemap",
    "options_removed_hydratable",
    "options_removed_loop_guard_timeout",
    "options_renamed_ssr_dom",
    "custom_element_props_identifier",
    "export_let_unused",
    "legacy_component_creation",
    "non_reactive_update",
    "perf_avoid_inline_class",
    "perf_avoid_nested_class",
    "reactive_declaration_invalid_placement",
    "reactive_declaration_module_script_dependency",
    "state_referenced_locally",
    "store_rune_conflict",
    "css_unused_selector",
    "attribute_avoid_is",
    "attribute_global_event_reference",
    "attribute_illegal_colon",
    "attribute_invalid_property_name",
    "attribute_quoted",
    "bind_invalid_each_rest",
    "block_empty",
    "component_name_lowercase",
    "element_implicitly_closed",
    "element_invalid_self_closing_tag",
    "event_directive_deprecated",
    "node_invalid_placement_ssr",
    "script_context_deprecated",
    "script_unknown_attribute",
    "slot_element_deprecated",
    "svelte_component_deprecated",
    "svelte_element_invalid_this",
    "svelte_self_deprecated",
    "await_waterfall",
    "await_reactivity_loss",
    "state_snapshot_uncloneable",
    "binding_property_non_reactive",
    "hydration_attribute_changed",
    "hydration_html_changed",
    "ownership_invalid_binding",
    "ownership_invalid_mutation",
];

/// The code the compiler suggests for `input`, a code that names no
/// warning: the compiler's `fuzzymatch` over [`IGNORE_CODES`].
fn fuzzymatch_known_code(input: &str) -> Option<&'static str> {
    crate::fuzzymatch::fuzzymatch(input, IGNORE_CODES)
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

#[cfg(test)]
mod tests {
    use super::{IGNORE_CODES, is_known_code};

    /// The ordered list names exactly the codes the ignore parser
    /// accepts, so a warning added to the catalog cannot miss it.
    #[test]
    fn ignore_codes_are_the_known_codes() {
        let catalog: Vec<&str> = crate::codes::CODES
            .iter()
            .copied()
            .chain(super::IGNORABLE_RUNTIME_WARNINGS.iter().copied())
            .filter(|c| is_known_code(c))
            .collect();
        for code in &catalog {
            assert!(
                IGNORE_CODES.contains(code),
                "{code} missing from IGNORE_CODES"
            );
        }
        for code in IGNORE_CODES {
            assert!(is_known_code(code), "{code} is no known code");
        }
        assert_eq!(catalog.len(), IGNORE_CODES.len());
    }
}
