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
use svn_parser::ast::{
    AttrValue, AttrValuePart, Attribute, Comment, DirectiveValue, Fragment, Node,
};

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
    /// `(node start, codes, codes that name no warning)` for every
    /// `svelte-ignore` comment attached as a leading comment, in
    /// comment order. The node's offset is script-local, matching oxc
    /// spans.
    leading: Vec<(u32, Vec<SmolStr>, Vec<BadIgnoreCode>)>,
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
    ///
    /// `prior` holds the codes of the comments the compiler read before
    /// this script (see [`prior_script_ignores`]); they lead the
    /// script's first node.
    ///
    /// `base` is where `content` starts in the file.
    pub(crate) fn build(
        program: &Program<'_>,
        content: &str,
        base: u32,
        runes: bool,
        prior: PriorIgnores,
    ) -> Self {
        let codes: Vec<(Vec<SmolStr>, Vec<BadIgnoreCode>)> = program
            .comments
            .iter()
            .map(|c| {
                content
                    .get(c.span.start as usize..c.span.end as usize)
                    .map(|text| {
                        (
                            comment_ignore_codes(text, runes),
                            comment_bad_codes(text, base + c.span.start, runes),
                        )
                    })
                    .unwrap_or_default()
            })
            .collect();
        let nothing =
            |(codes, bad): &(Vec<SmolStr>, Vec<BadIgnoreCode>)| codes.is_empty() && bad.is_empty();
        if prior.codes.is_empty() && prior.bad_codes.is_empty() && codes.iter().all(nothing) {
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
            first_node: None,
        };
        attacher.visit_program(program);
        let mut leading: Vec<(u32, Vec<SmolStr>, Vec<BadIgnoreCode>)> = Vec::new();
        if let Some(first) = attacher.first_node
            && !(prior.codes.is_empty() && prior.bad_codes.is_empty())
        {
            leading.push((first, prior.codes, prior.bad_codes));
        }
        leading.extend(attacher.leading.into_iter().filter_map(|(start, i)| {
            let (codes, bad) = codes.get(i).filter(|c| !nothing(c))?;
            Some((start, codes.clone(), bad.clone()))
        }));
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
        for (_, entry, _) in self.leading.iter().filter(|(s, _, _)| *s == node_start) {
            for c in entry {
                if !codes.contains(c) {
                    codes.push(c.clone());
                }
            }
        }
        codes
    }

    /// The codes that name no warning in the comments leading the node
    /// that starts at `node_start`. The compiler reads a node's leading
    /// comments when it visits the node, and warns about each then.
    pub(crate) fn leading_bad_codes(&self, node_start: u32) -> Vec<BadIgnoreCode> {
        self.leading
            .iter()
            .filter(|(s, _, _)| *s == node_start)
            .flat_map(|(_, _, bad)| bad.iter().cloned())
            .collect()
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
fn comment_bad_codes(text: &str, start: u32, runes: bool) -> Vec<BadIgnoreCode> {
    if !runes {
        return Vec::new();
    }
    let Some(body) = crate::scope_util::strip_comment_delimiters(text) else {
        return Vec::new();
    };
    let Some(rest) = body.trim_start().strip_prefix("svelte-ignore") else {
        return Vec::new();
    };
    let Some(space) = rest.chars().next().filter(|c| c.is_whitespace()) else {
        return Vec::new();
    };
    let rest = &rest[space.len_utf8()..];
    // `rest` is a tail of `body`, which is a slice of `text`.
    let base = start + (rest.as_ptr() as usize - text.as_ptr() as usize) as u32;
    let mut out = Vec::new();
    for (at, token, followed_by_comma) in runes_ignore_tokens(rest) {
        if !is_known_code(token) {
            let token_start = base + at as u32;
            let range = Range::new(token_start, token_start + token.len() as u32);
            let replacement = legacy_rename(token)
                .map(str::to_string)
                .unwrap_or_else(|| token.replace('-', "_"));
            out.push(if is_known_code(&replacement) {
                BadIgnoreCode {
                    code: Code::legacy_code,
                    message: messages::legacy_code(token, &replacement),
                    range,
                }
            } else {
                BadIgnoreCode {
                    code: Code::unknown_code,
                    message: messages::unknown_code(token, fuzzymatch_known_code(token)),
                    range,
                }
            });
        }
        if !followed_by_comma {
            break;
        }
    }
    out
}

/// The `svelte-ignore` codes a JavaScript comment's text (delimiters
/// included) carries; empty for any other comment.
fn comment_ignore_codes(text: &str, runes: bool) -> Vec<SmolStr> {
    crate::scope_util::strip_comment_delimiters(text)
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
    let mut comments: Vec<Range> = Vec::new();
    for &(content_start, program) in scripts {
        if content_start < before {
            comments.extend(
                program
                    .comments
                    .iter()
                    .map(|c| Range::new(content_start + c.span.start, content_start + c.span.end)),
            );
        }
    }
    if let Some(fragment) = fragment {
        collect_fragment_comments(fragment, source, before, &mut comments);
    }
    let mut prior = PriorIgnores::default();
    for range in comments {
        for code in comment_ignore_codes(range.slice(source), runes) {
            if !prior.codes.contains(&code) {
                prior.codes.push(code);
            }
        }
        // Read again as a leading comment of this script's first node,
        // a code that names no warning is warned about again.
        prior
            .bad_codes
            .extend(comment_bad_codes(range.slice(source), range.start, runes));
    }
    prior
}

/// The JavaScript comments in the template expressions and start tags
/// of the nodes in `fragment` that start before `before`.
fn collect_fragment_comments(fragment: &Fragment, source: &str, before: u32, out: &mut Vec<Range>) {
    for node in &fragment.nodes {
        if node.range().start >= before {
            break;
        }
        let mut expr = |range: Range| out.extend(expression_comments(source, range));
        match node {
            Node::Text(_) | Node::Comment(_) => {}
            Node::Interpolation(i) => expr(i.expression_range),
            Node::Element(e) => {
                attribute_comments(&e.attributes, source, out);
                collect_fragment_comments(&e.children, source, before, out);
            }
            Node::Component(c) => {
                attribute_comments(&c.attributes, source, out);
                collect_fragment_comments(&c.children, source, before, out);
            }
            Node::SvelteElement(e) => {
                attribute_comments(&e.attributes, source, out);
                collect_fragment_comments(&e.children, source, before, out);
            }
            Node::IfBlock(b) => {
                expr(b.condition_range);
                collect_fragment_comments(&b.consequent, source, before, out);
                for arm in &b.elseif_arms {
                    out.extend(expression_comments(source, arm.condition_range));
                    collect_fragment_comments(&arm.body, source, before, out);
                }
                if let Some(alt) = &b.alternate {
                    collect_fragment_comments(alt, source, before, out);
                }
            }
            Node::EachBlock(b) => {
                // Every comment the compiler's reads of the tag lexed.
                out.extend(b.open_comments.iter().copied());
                collect_fragment_comments(&b.body, source, before, out);
                if let Some(alt) = &b.alternate {
                    collect_fragment_comments(alt, source, before, out);
                }
            }
            Node::AwaitBlock(b) => {
                expr(b.expression_range);
                if let Some(pending) = &b.pending {
                    collect_fragment_comments(pending, source, before, out);
                }
                if let Some(then) = &b.then_branch {
                    if let Some(range) = then.context_range {
                        out.extend(js_comments_in(source, range));
                    }
                    collect_fragment_comments(&then.body, source, before, out);
                }
                if let Some(catch) = &b.catch_branch {
                    if let Some(range) = catch.context_range {
                        out.extend(js_comments_in(source, range));
                    }
                    collect_fragment_comments(&catch.body, source, before, out);
                }
            }
            Node::KeyBlock(b) => {
                expr(b.expression_range);
                collect_fragment_comments(&b.body, source, before, out);
            }
            Node::SnippetBlock(b) => {
                // Read whole: no expression ends early here.
                if let Some(generics) = b.generics_range {
                    out.extend(js_comments_in(source, generics));
                }
                out.extend(js_comments_in(source, b.parameters_range));
                collect_fragment_comments(&b.body, source, before, out);
            }
        }
    }
}

fn attribute_comments(attributes: &[Attribute], source: &str, out: &mut Vec<Range>) {
    let parts = |value: &AttrValue, out: &mut Vec<Range>| {
        for part in &value.parts {
            if let AttrValuePart::Expression {
                expression_range, ..
            } = part
            {
                out.extend(expression_comments(source, *expression_range));
            }
        }
    };
    for attr in attributes {
        match attr {
            Attribute::Plain(p) => {
                if let Some(value) = &p.value {
                    parts(value, out);
                }
            }
            Attribute::Expression(e) => {
                out.extend(expression_comments(source, e.expression_range));
            }
            Attribute::Shorthand(_) => {}
            Attribute::Spread(s) => out.extend(expression_comments(source, s.expression_range)),
            Attribute::Attach(a) => out.extend(expression_comments(source, a.expression_range)),
            Attribute::Directive(d) => match &d.value {
                Some(DirectiveValue::Expression {
                    expression_range, ..
                }) => out.extend(expression_comments(source, *expression_range)),
                Some(DirectiveValue::BindPair {
                    getter_range,
                    setter_range,
                    ..
                }) => out.extend(expression_comments(
                    source,
                    Range::new(getter_range.start, setter_range.end),
                )),
                Some(DirectiveValue::Quoted(value)) => parts(value, out),
                None => {}
            },
            Attribute::Comment(c) => out.push(c.range),
        }
    }
}

/// The comments in the JavaScript at `range` — an expression, a
/// declaration's body, a binding pattern or a parameter list — as oxc
/// reads them. Each shape is tried in a wrapper that makes it a
/// program; the first that parses cleanly has lexed the whole text.
/// The comments acorn lexes reading the expression of a tag or an
/// attribute value that starts at `range`: those inside it, and those
/// between its end and the `}` that closes the tag (the node's range
/// stops at the expression, but acorn looks at the next token).
fn expression_comments(source: &str, range: Range) -> Vec<Range> {
    let close = svn_parser::find_mustache_end(source, range.start)
        .unwrap_or(range.end)
        .max(range.end);
    js_comments_in(source, Range::new(range.start, close))
}

fn js_comments_in(source: &str, range: Range) -> Vec<Range> {
    let Some(text) = source.get(range.start as usize..range.end as usize) else {
        return Vec::new();
    };
    if !text.contains("/*") && !text.contains("//") {
        return Vec::new();
    }
    const WRAPPERS: &[(&str, &str)] = &[
        ("(", "\n)"),
        ("let ", "\n;"),
        ("let ", "\n= 0;"),
        ("(", "\n) => 0"),
        ("<", "\n>() => 0"),
    ];
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    for (prefix, suffix) in WRAPPERS {
        let wrapped = format!("{prefix}{text}{suffix}");
        let alloc = oxc_allocator::Allocator::default();
        let parsed = oxc_parser::Parser::new(&alloc, &wrapped, source_type).parse();
        if parsed.diagnostics.is_empty() && !parsed.fatal_error {
            let shift = prefix.len() as u32;
            return parsed
                .program
                .comments
                .iter()
                .filter(|c| c.span.start >= shift)
                .map(|c| {
                    Range::new(
                        range.start + c.span.start - shift,
                        range.start + c.span.end - shift,
                    )
                })
                .collect();
        }
    }
    Vec::new()
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
    /// Start of the first node inside the program.
    first_node: Option<u32>,
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
        if self.first_node.is_none() && !matches!(kind, AstKind::Program(_)) {
            self.first_node = Some(span.start);
        }
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
