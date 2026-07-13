//! Template-level parser.
//!
//! Parses the template fragments recovered by the structural section parser
//! into a proper [`Fragment`] AST. Scope:
//!
//! - Text and `{expression}` mustache interpolations
//! - `<!-- comments -->`
//! - Elements / components / `<svelte:*>` with attributes and directives
//! - Void + self-closing elements
//! - Control-flow blocks (`{#if}`/`{#each}`/`{#await}`/`{#key}`/`{#snippet}`)
//!   and their branches
//! - Tags (`{@html}`/`{@const}`/`{@render}`/`{@debug}`/`{@attach}`)
//!
//! All scans are bounded to the current fragment (`fragment_end`) so a
//! malformed/unterminated construct can't consume into a following
//! template run or script/style section.

use smol_str::SmolStr;
use svn_core::Range;

use crate::ast::{
    CatchBranch, Comment, Component, Element, ElseIfArm, Fragment, Interpolation,
    InterpolationKind, Node, SvelteElement, SvelteElementKind, Text, ThenBranch, is_component_tag,
    is_void_element,
};
use crate::attributes::parse_attributes;
use crate::blocks::{
    AwaitShortForm, BlockTerminator, build_await_block, build_each_block, build_if_block,
    build_key_block, build_snippet_block, parse_await_header, parse_each_header, parse_if_header,
    parse_key_header, parse_snippet_header, peek_and_consume_terminator,
};
use crate::error::ParseError;
use crate::html5::closing_tag_omitted;
use crate::mustache::find_mustache_end;
use crate::scanner::Scanner;

/// Parse one template fragment out of the source string.
///
/// `range` is the byte range within `source` that contains template text.
/// The returned [`Fragment`] has `range == range`.
pub fn parse_template(source: &str, range: Range) -> (Fragment, Vec<ParseError>) {
    let mut parser = TemplateParser::new(source, range);
    let mut nodes = Vec::new();
    // A terminator surfacing at top level is a stray `{/...}` / `{:...}`
    // with no open block — upstream fires block_unexpected_close /
    // block_invalid_continuation_placement. Record the error and resume
    // so the rest of the template isn't silently dropped.
    loop {
        let (mut more, stop) = parser.parse_fragment_until(None);
        nodes.append(&mut more);
        match stop {
            None => break,
            Some(term) => parser.push_stray_terminator_error(&term),
        }
    }
    (
        Fragment {
            nodes,
            range: parser.fragment_range,
        },
        parser.errors,
    )
}

/// Parse every template run in a source file and merge them into one
/// fragment whose `range` spans the first-start to last-end. Gaps (where
/// `<script>`/`<style>` blocks sit) are NOT represented in the fragment;
/// the emitter re-inserts script/style at their original locations.
pub fn parse_all_template_runs(source: &str, runs: &[Range]) -> (Fragment, Vec<ParseError>) {
    let mut all_nodes = Vec::new();
    let mut all_errors = Vec::new();
    let mut combined_start = u32::MAX;
    let mut combined_end = 0;
    for run in runs {
        let (frag, errs) = parse_template(source, *run);
        if !run.is_empty() {
            combined_start = combined_start.min(run.start);
            combined_end = combined_end.max(run.end);
        }
        all_nodes.extend(frag.nodes);
        all_errors.extend(errs);
    }
    let range = if combined_start <= combined_end && combined_start != u32::MAX {
        Range::new(combined_start, combined_end)
    } else {
        Range::new(0, 0)
    };
    (
        Fragment {
            nodes: all_nodes,
            range,
        },
        all_errors,
    )
}

/// Frame in the open-element stack, mirroring the roles upstream's
/// parser stack plays in the implicit-close walks
/// (`1-parse/state/element.js`).
enum OpenFrame {
    /// Plain HTML element — may be implicitly closed (upstream pops it
    /// with only the `element_implicitly_closed` warning).
    Element(SmolStr),
    /// Component or `svelte:*` element — a valid target for an ancestor
    /// closing tag, but never implicitly closable itself (upstream errors
    /// `element_invalid_closing_tag` instead of popping).
    Component(SmolStr),
    /// `{#...}` block boundary — the ancestor walk never crosses it
    /// (upstream errors `block_unexpected_close` /
    /// `element_invalid_closing_tag`).
    Block,
}

struct TemplateParser<'src> {
    scanner: Scanner<'src>,
    fragment_range: Range,
    fragment_end: u32,
    errors: Vec<ParseError>,
    /// Source span of the most recently consumed block terminator
    /// (`{:...}` / `{/...}`). [`Self::parse_fragment_until`] records it as
    /// it consumes the terminator so callers that receive one where none
    /// is expected can report an accurately-positioned error.
    last_terminator_range: Range,
    /// Open elements/blocks enclosing the current parse position, mirroring
    /// the recursion. Consulted by the HTML implicit-close checks.
    open_elements: Vec<OpenFrame>,
    /// Set when [`Self::parse_fragment_until`] stops because the current
    /// element was implicitly closed (HTML `closing_tag_omitted` rules or
    /// an ancestor's closing tag). Only ever set while a `stop_element` is
    /// active, so only [`Self::parse_children_until`] consumes it: no
    /// closing tag is expected and no error is recorded — upstream emits
    /// just the `element_implicitly_closed` warning (svn-lint's concern).
    pending_auto_close: bool,
}

impl<'src> TemplateParser<'src> {
    fn new(source: &'src str, range: Range) -> Self {
        let mut scanner = Scanner::new(source);
        scanner.set_pos(range.start);
        Self {
            scanner,
            fragment_range: range,
            fragment_end: range.end,
            errors: Vec::new(),
            last_terminator_range: Range::new(range.start, range.start),
            open_elements: Vec::new(),
            pending_auto_close: false,
        }
    }

    /// Report a block terminator that surfaced where no block is open —
    /// at template top level or inside an element's children. Mirrors
    /// upstream: a stray `{/...}` fires `block_unexpected_close`; a stray
    /// `{:...}` fires `block_invalid_continuation_placement`.
    fn push_stray_terminator_error(&mut self, term: &BlockTerminator) {
        let range = self.last_terminator_range;
        self.errors.push(match term {
            BlockTerminator::Close { .. } => ParseError::UnexpectedBlockClose { range },
            _ => ParseError::InvalidBlockContinuation { range },
        });
    }

    fn at_fragment_end(&self) -> bool {
        self.scanner.pos() >= self.fragment_end || self.scanner.eof()
    }

    /// [`find_mustache_end`] clamped to this fragment. The scanner spans
    /// the whole source, so an unterminated mustache would otherwise
    /// find a `}` belonging to a following template run or script/style
    /// section, producing a node whose range bleeds past `fragment_end`.
    /// A match at/after `fragment_end` means the mustache is unterminated
    /// *within this fragment* — report `None` so the caller's recovery
    /// path (which clamps to `fragment_end`) runs instead.
    fn find_mustache_end_in_fragment(&self, expr_start: u32) -> Option<u32> {
        find_mustache_end(self.scanner.source(), expr_start).filter(|&end| end < self.fragment_end)
    }

    /// Peek for a Svelte 5 declaration-tag opener at the current `{`:
    /// `{const`/`{let` followed by a word boundary. Returns the kind and
    /// the keyword byte length (`5` for `const`, `3` for `let`) on a
    /// match, else `None` (in which case the `{…}` is a plain expression
    /// — `{constant}`, `{letter}`, etc.). Mirrors the Svelte compiler's
    /// `/(?:let|const)\b/y` in `phases/1-parse/state/tag.js`: the keyword
    /// must not run into a longer identifier.
    fn peek_declaration_tag(&self) -> Option<(InterpolationKind, u32)> {
        let src = self.scanner.source();
        let after_brace = (self.scanner.pos() + 1) as usize;
        let rest = src.get(after_brace..)?;
        let (kind, kw_len) = if rest.starts_with("const") {
            (InterpolationKind::DeclConst, 5usize)
        } else if rest.starts_with("let") {
            (InterpolationKind::DeclLet, 3usize)
        } else {
            return None;
        };
        // Word boundary: the byte after the keyword must not continue an
        // identifier (so `{constant}` / `{letter}` stay plain expressions).
        match rest.as_bytes().get(kw_len).copied() {
            Some(b) if b.is_ascii_alphanumeric() || b == b'_' || b == b'$' => None,
            _ => Some((kind, kw_len as u32)),
        }
    }

    /// Parse a sequence of nodes until a stop condition is hit. Returns the
    /// parsed nodes and *why* parsing stopped.
    ///
    /// Stop conditions:
    /// - End of fragment / EOF — returns `None`.
    /// - `</stop_element>` if `stop_element` is `Some(tag)` — the caller
    ///   consumes it.
    /// - A block-level terminator `{:...}` or `{/...}` — the caller
    ///   dispatches on it (e.g. the `{#if}` handler uses `{:else}` to start
    ///   the alternate branch).
    fn parse_fragment_until(
        &mut self,
        stop_element: Option<&str>,
    ) -> (Vec<Node>, Option<BlockTerminator>) {
        let mut nodes = Vec::new();
        while !self.at_fragment_end() {
            // Block-level terminator: `{:...}` or `{/...}`.
            if self.scanner.starts_with("{:") || self.scanner.starts_with("{/") {
                let term_start = self.scanner.pos();
                if let Some(term) = peek_and_consume_terminator(&mut self.scanner, &mut self.errors)
                {
                    self.last_terminator_range = Range::new(term_start, self.scanner.pos());
                    return (nodes, Some(term));
                }
                // Malformed terminator — error was recorded; advance to
                // avoid infinite loop.
                self.scanner.advance_byte();
                continue;
            }

            // Block header: `{#if}`, `{#each}`, `{#await}`, `{#key}`, `{#snippet}`.
            if self.scanner.starts_with("{#") {
                if let Some(node) = self.parse_block() {
                    nodes.push(node);
                }
                continue;
            }

            // Tag (`{@html}` / `{@const}` / `{@render}` / `{@debug}` / `{@attach}`).
            //
            // Real Svelte semantics differ per tag (`@html` injects raw HTML,
            // `@render` invokes a snippet, `@const` declares a local in
            // template scope, `@debug` stops in the debugger, `@attach`
            // attaches behavior). For type-checking purposes the only thing
            // that matters here is what *identifiers* the tag's body
            // references — those are real value reads from the script's
            // scope and need to flow into our void-ref pass.
            //
            // We model each tag as an `Interpolation` whose expression range
            // covers the body after the tag keyword. This is a deliberate
            // semantic shortcut: it isn't quite right for `@const name = expr`
            // (where `name` is a new binding, not a reference), but the
            // template-ref pass intersects with script bindings, so a `@const`
            // local that doesn't shadow a script local just gets dropped.
            if self.scanner.starts_with("{@") {
                let start = self.scanner.pos();
                match self.find_mustache_end_in_fragment(self.scanner.pos() + 1) {
                    Some(end) => {
                        let body_start = self.scanner.pos() + 2; // past `{@`
                        let src = self.scanner.source();
                        let bytes = src.as_bytes();
                        // Read the tag keyword (alpha chars) into a slice
                        // so we can classify it (@const vs anything else).
                        let keyword_start = body_start as usize;
                        let mut p = keyword_start;
                        while p < end as usize && bytes[p].is_ascii_alphabetic() {
                            p += 1;
                        }
                        let keyword = &src[keyword_start..p];
                        let kind = match keyword {
                            "const" => InterpolationKind::AtConst,
                            "html" => InterpolationKind::AtHtml,
                            "render" => InterpolationKind::AtRender,
                            "debug" => InterpolationKind::AtDebug,
                            _ => InterpolationKind::AtTag,
                        };
                        // Skip whitespace after the keyword.
                        while p < end as usize && bytes[p].is_ascii_whitespace() {
                            p += 1;
                        }
                        let expr_range = Range::new(p as u32, end);
                        self.scanner.set_pos(end + 1);
                        nodes.push(Node::Interpolation(Interpolation {
                            kind,
                            expression_range: expr_range,
                            range: Range::new(start, end + 1),
                        }));
                    }
                    None => {
                        self.errors.push(ParseError::UnterminatedMustache {
                            range: Range::new(start, self.fragment_end),
                        });
                        self.scanner.set_pos(self.fragment_end);
                    }
                }
                continue;
            }

            // Svelte 5 declaration tag: `{const …}` / `{let …}` (bare,
            // no `@`). Distinguished from a plain `{constant}` /
            // `{letter}` expression by requiring a word boundary after
            // the keyword. Mirrors the Svelte compiler's
            // `/(?:let|const)\b/y` (phases/1-parse/state/tag.js). Unlike
            // `{@const}`, a declaration tag is freely placeable, so it
            // carries no parent-placement restriction.
            //
            // Modelled as an `Interpolation` whose `expression_range`
            // covers the declaration body after the keyword + whitespace
            // (e.g. `foo: number = 1` for `{let foo: number = 1}`), the
            // same convention `{@const}` uses — emit prepends the right
            // keyword and the analyze pass reuses `extract_at_const_bindings`.
            if let Some((kind, kw_len)) = self.peek_declaration_tag() {
                let start = self.scanner.pos();
                match self.find_mustache_end_in_fragment(self.scanner.pos() + 1) {
                    Some(end) => {
                        let src = self.scanner.source();
                        let bytes = src.as_bytes();
                        // Skip whitespace after the `const`/`let` keyword.
                        let mut p = (start + 1 + kw_len) as usize;
                        while p < end as usize && bytes[p].is_ascii_whitespace() {
                            p += 1;
                        }
                        let expr_range = Range::new(p as u32, end);
                        self.scanner.set_pos(end + 1);
                        nodes.push(Node::Interpolation(Interpolation {
                            kind,
                            expression_range: expr_range,
                            range: Range::new(start, end + 1),
                        }));
                    }
                    None => {
                        self.errors.push(ParseError::UnterminatedMustache {
                            range: Range::new(start, self.fragment_end),
                        });
                        self.scanner.set_pos(self.fragment_end);
                    }
                }
                continue;
            }

            // `</TAG>` closing tag.
            if self.scanner.starts_with("</") {
                if let Some(tag) = stop_element {
                    if self.peek_closing_tag_matches(tag) {
                        return (nodes, None);
                    }
                    // A closing tag for an open ancestor implicitly closes
                    // this element: upstream pops plain-element parents
                    // with only the element_implicitly_closed warning
                    // until the names match (`<div><p>text</div>`). Left
                    // unconsumed — the matching ancestor's own frame
                    // consumes it.
                    if self.closing_tag_matches_open_ancestor() {
                        self.pending_auto_close = true;
                        return (nodes, None);
                    }
                }
                self.errors.push(ParseError::MalformedOpenTag {
                    range: Range::new(self.scanner.pos(), self.scanner.pos() + 2),
                });
                self.scanner.advance(2);
                continue;
            }

            if self.scanner.starts_with("<!--") {
                nodes.push(self.parse_comment());
                continue;
            }

            if self.scanner.peek_byte() == Some(b'<') {
                // A new opening tag implicitly closes the current element
                // when HTML omits its closing tag (`<li>a<li>` — upstream
                // checks closing_tag_omitted(parent, tag) before pushing
                // the new element). Left unconsumed — it parses as the
                // parent's next child.
                if let Some(stop) = stop_element
                    && self.open_tag_implicitly_closes(stop)
                {
                    self.pending_auto_close = true;
                    return (nodes, None);
                }
                if let Some(node) = self.parse_element_or_component() {
                    nodes.push(node);
                }
                continue;
            }

            if self.scanner.peek_byte() == Some(b'{') {
                if let Some(node) = self.parse_interpolation() {
                    nodes.push(node);
                } else {
                    self.scanner.advance_byte();
                }
                continue;
            }

            nodes.push(self.parse_text());
        }
        (nodes, None)
    }

    /// Dispatch on the block name after `{#` and build the correct block node.
    fn parse_block(&mut self) -> Option<Node> {
        let block_start = self.scanner.pos();
        debug_assert!(self.scanner.starts_with("{#"));
        self.scanner.advance(2);

        let name_start = self.scanner.pos();
        while let Some(b) = self.scanner.peek_byte() {
            if b.is_ascii_alphabetic() {
                self.scanner.advance_byte();
            } else {
                break;
            }
        }
        let name_end = self.scanner.pos();
        let name = &self.scanner.source()[name_start as usize..name_end as usize];

        // Known blocks bracket their bodies with a Block frame: the
        // implicit-close ancestor walk must not cross a block boundary
        // (upstream: block nodes aren't RegularElements, so the close
        // walk errors instead of popping through them).
        let parse_known = |p: &mut Self, kind: fn(&mut Self, u32) -> Option<Node>| {
            p.open_elements.push(OpenFrame::Block);
            let node = kind(p, block_start);
            p.open_elements.pop();
            node
        };
        match name {
            "if" => parse_known(self, Self::parse_if_block),
            "each" => parse_known(self, Self::parse_each_block),
            "await" => parse_known(self, Self::parse_await_block),
            "key" => parse_known(self, Self::parse_key_block),
            "snippet" => parse_known(self, Self::parse_snippet_block),
            other => {
                self.errors.push(ParseError::UnsupportedBlock {
                    range: Range::new(block_start, self.scanner.pos()),
                });
                // Skip to the end of this opening mustache and beyond to the
                // matching `{/other}` if we can find one — else bail.
                if let Some(end) = self.find_mustache_end_in_fragment(self.scanner.pos()) {
                    self.scanner.set_pos(end + 1);
                }
                let needle = format!("{{/{other}}}");
                // Only jump to a matching close that lies within this
                // fragment — a `{/other}` in a later run isn't ours.
                if let Some(pos) = self
                    .scanner
                    .find(needle.as_bytes())
                    .filter(|&pos| pos < self.fragment_end)
                {
                    self.scanner.set_pos(pos + needle.len() as u32);
                }
                None
            }
        }
    }

    fn parse_if_block(&mut self, block_start: u32) -> Option<Node> {
        let condition_range = parse_if_header(&mut self.scanner, &mut self.errors)?;
        let (consequent_nodes, term) = self.parse_fragment_until(None);
        let consequent = Fragment {
            nodes: consequent_nodes,
            range: Range::new(condition_range.end + 1, self.scanner.pos()),
        };

        let mut elseif_arms: Vec<ElseIfArm> = Vec::new();
        let mut alternate: Option<Fragment> = None;
        let mut next = term;

        loop {
            match next {
                Some(BlockTerminator::Close { tag }) if tag == "if" => break,
                Some(BlockTerminator::ElseIf { condition_range }) => {
                    let body_start = self.scanner.pos();
                    let (body_nodes, t) = self.parse_fragment_until(None);
                    elseif_arms.push(ElseIfArm {
                        condition_range,
                        body: Fragment {
                            nodes: body_nodes,
                            range: Range::new(body_start, self.scanner.pos()),
                        },
                    });
                    next = t;
                }
                Some(BlockTerminator::Else) => {
                    let body_start = self.scanner.pos();
                    let (body_nodes, t) = self.parse_fragment_until(None);
                    alternate = Some(Fragment {
                        nodes: body_nodes,
                        range: Range::new(body_start, self.scanner.pos()),
                    });
                    next = t;
                }
                _ => {
                    self.errors.push(ParseError::UnterminatedElement {
                        name: "if".to_string(),
                        range: Range::new(block_start, self.scanner.pos()),
                    });
                    break;
                }
            }
        }

        Some(Node::IfBlock(build_if_block(
            condition_range,
            consequent,
            elseif_arms,
            alternate,
            block_start,
            self.scanner.pos(),
        )))
    }

    fn parse_each_block(&mut self, block_start: u32) -> Option<Node> {
        let (expr_range, as_clause) = parse_each_header(&mut self.scanner, &mut self.errors)?;
        let (body_nodes, term) = self.parse_fragment_until(None);
        let body = Fragment {
            nodes: body_nodes,
            range: Range::new(expr_range.end + 1, self.scanner.pos()),
        };

        let mut alternate: Option<Fragment> = None;
        match term {
            Some(BlockTerminator::Close { tag }) if tag == "each" => {}
            Some(BlockTerminator::Else) => {
                let alt_start = self.scanner.pos();
                let (alt_nodes, t2) = self.parse_fragment_until(None);
                alternate = Some(Fragment {
                    nodes: alt_nodes,
                    range: Range::new(alt_start, self.scanner.pos()),
                });
                match t2 {
                    Some(BlockTerminator::Close { tag }) if tag == "each" => {}
                    _ => {
                        self.errors.push(ParseError::UnterminatedElement {
                            name: "each".to_string(),
                            range: Range::new(block_start, self.scanner.pos()),
                        });
                    }
                }
            }
            _ => {
                self.errors.push(ParseError::UnterminatedElement {
                    name: "each".to_string(),
                    range: Range::new(block_start, self.scanner.pos()),
                });
            }
        }

        Some(Node::EachBlock(build_each_block(
            expr_range,
            as_clause,
            body,
            alternate,
            block_start,
            self.scanner.pos(),
        )))
    }

    fn parse_await_block(&mut self, block_start: u32) -> Option<Node> {
        let (expr_range, short) = parse_await_header(&mut self.scanner, &mut self.errors)?;

        let (mut pending, mut then_branch, mut catch_branch) = (None, None, None);

        match short {
            AwaitShortForm::Then(ctx) => {
                // `{#await p then v}` — body is the then-branch directly,
                // BUT `{:catch}` is still allowed afterward in Svelte's
                // grammar. So parse until either `:catch` or `{/await}`.
                let then_start = self.scanner.pos();
                let (body, mut term) = self.parse_fragment_until(None);
                then_branch = Some(ThenBranch {
                    context_range: ctx,
                    body: Fragment {
                        nodes: body,
                        range: Range::new(then_start, self.scanner.pos()),
                    },
                });
                if let Some(BlockTerminator::Catch { context_range }) = &term {
                    let cctx = *context_range;
                    let catch_start = self.scanner.pos();
                    let (catch_nodes, t2) = self.parse_fragment_until(None);
                    catch_branch = Some(CatchBranch {
                        context_range: cctx,
                        body: Fragment {
                            nodes: catch_nodes,
                            range: Range::new(catch_start, self.scanner.pos()),
                        },
                    });
                    term = t2;
                }
                self.finish_await_block(term, block_start);
            }
            AwaitShortForm::Catch(ctx) => {
                let catch_start = self.scanner.pos();
                let (body, t) = self.parse_fragment_until(None);
                catch_branch = Some(CatchBranch {
                    context_range: ctx,
                    body: Fragment {
                        nodes: body,
                        range: Range::new(catch_start, self.scanner.pos()),
                    },
                });
                self.finish_await_block(t, block_start);
            }
            AwaitShortForm::None => {
                // Full form: pending, then optional :then, optional :catch, {/await}.
                let pending_start = self.scanner.pos();
                let (pending_nodes, mut term) = self.parse_fragment_until(None);
                pending = Some(Fragment {
                    nodes: pending_nodes,
                    range: Range::new(pending_start, self.scanner.pos()),
                });
                // :then?
                if let Some(BlockTerminator::Then { context_range }) = &term {
                    let ctx = *context_range;
                    let then_start = self.scanner.pos();
                    let (then_nodes, t2) = self.parse_fragment_until(None);
                    then_branch = Some(ThenBranch {
                        context_range: ctx,
                        body: Fragment {
                            nodes: then_nodes,
                            range: Range::new(then_start, self.scanner.pos()),
                        },
                    });
                    term = t2;
                }
                // :catch?
                if let Some(BlockTerminator::Catch { context_range }) = &term {
                    let ctx = *context_range;
                    let catch_start = self.scanner.pos();
                    let (catch_nodes, t2) = self.parse_fragment_until(None);
                    catch_branch = Some(CatchBranch {
                        context_range: ctx,
                        body: Fragment {
                            nodes: catch_nodes,
                            range: Range::new(catch_start, self.scanner.pos()),
                        },
                    });
                    term = t2;
                }
                self.finish_await_block(term, block_start);
            }
        }

        Some(Node::AwaitBlock(build_await_block(
            expr_range,
            pending,
            then_branch,
            catch_branch,
            block_start,
            self.scanner.pos(),
        )))
    }

    fn finish_await_block(&mut self, term: Option<BlockTerminator>, block_start: u32) {
        match term {
            Some(BlockTerminator::Close { tag }) if tag == "await" => {}
            _ => {
                self.errors.push(ParseError::UnterminatedElement {
                    name: "await".to_string(),
                    range: Range::new(block_start, self.scanner.pos()),
                });
            }
        }
    }

    fn parse_key_block(&mut self, block_start: u32) -> Option<Node> {
        let expr_range = parse_key_header(&mut self.scanner, &mut self.errors)?;
        let (body_nodes, term) = self.parse_fragment_until(None);
        let body = Fragment {
            nodes: body_nodes,
            range: Range::new(expr_range.end + 1, self.scanner.pos()),
        };
        match term {
            Some(BlockTerminator::Close { tag }) if tag == "key" => {}
            _ => {
                self.errors.push(ParseError::UnterminatedElement {
                    name: "key".to_string(),
                    range: Range::new(block_start, self.scanner.pos()),
                });
            }
        }
        Some(Node::KeyBlock(build_key_block(
            expr_range,
            body,
            block_start,
            self.scanner.pos(),
        )))
    }

    fn parse_snippet_block(&mut self, block_start: u32) -> Option<Node> {
        let (name, params_range) = parse_snippet_header(&mut self.scanner, &mut self.errors)?;
        let (body_nodes, term) = self.parse_fragment_until(None);
        let body = Fragment {
            nodes: body_nodes,
            range: Range::new(params_range.end + 1, self.scanner.pos()),
        };
        match term {
            Some(BlockTerminator::Close { tag }) if tag == "snippet" => {}
            _ => {
                self.errors.push(ParseError::UnterminatedElement {
                    name: "snippet".to_string(),
                    range: Range::new(block_start, self.scanner.pos()),
                });
            }
        }
        Some(Node::SnippetBlock(build_snippet_block(
            name,
            params_range,
            body,
            block_start,
            self.scanner.pos(),
        )))
    }

    fn parse_text(&mut self) -> Node {
        let start = self.scanner.pos();
        while !self.at_fragment_end() {
            let b = match self.scanner.peek_byte() {
                Some(b) => b,
                None => break,
            };
            if b == b'<' || b == b'{' {
                break;
            }
            self.scanner.advance_char();
        }
        let end = self.scanner.pos();
        Node::Text(Text {
            range: Range::new(start, end),
        })
    }

    fn parse_comment(&mut self) -> Node {
        let start = self.scanner.pos();
        debug_assert!(self.scanner.starts_with("<!--"));
        self.scanner.advance(4);
        let body_start = self.scanner.pos();

        // Clamp to the fragment: a `-->` belonging to a later run /
        // section must not be treated as this comment's terminator
        // (it would over-consume into following content).
        let body_end = match self
            .scanner
            .find(b"-->")
            .filter(|&pos| pos < self.fragment_end)
        {
            Some(pos) => pos,
            None => {
                // Unterminated — treat everything to fragment end as the body.
                let end = self.fragment_end;
                self.errors.push(ParseError::UnterminatedComment {
                    range: Range::new(start, end),
                });
                self.scanner.set_pos(end);
                return Node::Comment(Comment {
                    data_range: Range::new(body_start, end),
                    range: Range::new(start, end),
                });
            }
        };

        self.scanner.set_pos(body_end);
        self.scanner.advance(3); // past "-->"
        Node::Comment(Comment {
            data_range: Range::new(body_start, body_end),
            range: Range::new(start, self.scanner.pos()),
        })
    }

    fn parse_interpolation(&mut self) -> Option<Node> {
        let start = self.scanner.pos();
        debug_assert_eq!(self.scanner.peek_byte(), Some(b'{'));
        self.scanner.advance_byte();
        let expr_start = self.scanner.pos();

        match self.find_mustache_end_in_fragment(expr_start) {
            Some(end) => {
                let expr_range = Range::new(expr_start, end);
                self.scanner.set_pos(end + 1);
                Some(Node::Interpolation(Interpolation {
                    kind: InterpolationKind::Expression,
                    expression_range: expr_range,
                    range: Range::new(start, self.scanner.pos()),
                }))
            }
            None => {
                self.errors.push(ParseError::UnterminatedMustache {
                    range: Range::new(start, self.fragment_end),
                });
                self.scanner.set_pos(self.fragment_end);
                None
            }
        }
    }

    fn parse_element_or_component(&mut self) -> Option<Node> {
        let tag_start = self.scanner.pos();
        debug_assert_eq!(self.scanner.peek_byte(), Some(b'<'));
        self.scanner.advance_byte();

        let name_start = self.scanner.pos();
        // `<!NAME` is a doctype-shaped tag (upstream `is_valid_element_name`
        // admits `/^![a-zA-Z]+$/`, so `<!DOCTYPE html>` parses as a void
        // element named `!DOCTYPE`). Consume the `!` and let the regular
        // name scan read the letters. `<!--` comments were dispatched
        // before this point.
        let doctype_shaped = self.scanner.peek_byte() == Some(b'!')
            && self
                .scanner
                .peek_byte_at(1)
                .is_some_and(|b| b.is_ascii_alphabetic());
        if doctype_shaped {
            self.scanner.advance_byte();
        } else if !self
            .scanner
            .peek_byte()
            .map(|b| b.is_ascii_alphabetic() || b >= 0x80)
            .unwrap_or(false)
        {
            // Not a valid element start — record error, skip `<`.
            self.errors.push(ParseError::MalformedOpenTag {
                range: Range::new(tag_start, self.scanner.pos()),
            });
            return None;
        }

        while let Some(b) = self.scanner.peek_byte() {
            if b.is_ascii_alphanumeric() || b >= 0x80 || matches!(b, b':' | b'-' | b'_' | b'.') {
                self.scanner.advance_byte();
            } else {
                break;
            }
        }
        let name_end = self.scanner.pos();
        let name: SmolStr = self.scanner.source()[name_start as usize..name_end as usize].into();

        // Parse attributes into real AST nodes.
        let attributes = parse_attributes(&mut self.scanner, self.fragment_end, &mut self.errors);

        // Consume `>` or `/>`.
        let (self_closing, open_tag_end) = self.finish_opening_tag()?;

        // Distinguish element kinds.
        if let Some(suffix) = name.strip_prefix("svelte:") {
            let Some(kind) = SvelteElementKind::parse(suffix) else {
                self.errors.push(ParseError::UnknownSvelteElement {
                    name: name.to_string(),
                    range: Range::new(tag_start, open_tag_end),
                });
                return None;
            };
            let children = if self_closing {
                Fragment::default()
            } else {
                self.parse_children_until(&name, tag_start, open_tag_end)?
            };
            return Some(Node::SvelteElement(SvelteElement {
                kind,
                attributes,
                children,
                self_closing,
                range: Range::new(tag_start, self.scanner.pos()),
            }));
        }

        if is_component_tag(&name) {
            let children = if self_closing {
                Fragment::default()
            } else {
                self.parse_children_until(&name, tag_start, open_tag_end)?
            };
            return Some(Node::Component(Component {
                name,
                attributes,
                children,
                self_closing,
                range: Range::new(tag_start, self.scanner.pos()),
            }));
        }

        // Normal HTML element.
        let is_void = is_void_element(&name);
        let children = if self_closing || is_void {
            Fragment::default()
        } else if name.eq_ignore_ascii_case("style") || name.eq_ignore_ascii_case("script") {
            // Raw-text both `style` AND nested `script` (e.g. a JSON-LD
            // `<script type="application/ld+json">` in `<svelte:head>`).
            // Svelte's parser blanks `<script>` bodies verbatim
            // (svelte2tsx htmlxparser `blankVerbatimContent`); without
            // this, `{...}` in a JSON-LD block parses as a mustache
            // interpolation and fires phantom diagnostics. `textarea`/
            // `title` are NOT raw-text — Svelte interpolates mustaches
            // inside them.
            self.parse_raw_text_children_until(&name, tag_start, open_tag_end)?
        } else {
            self.parse_children_until(&name, tag_start, open_tag_end)?
        };

        Some(Node::Element(Element {
            name,
            attributes,
            children,
            self_closing: self_closing || is_void,
            range: Range::new(tag_start, self.scanner.pos()),
        }))
    }

    /// After [`parse_attributes`] returns, the scanner points at `>` or `/`.
    /// Consume the closing delimiter and return `(self_closing, end_pos)`.
    fn finish_opening_tag(&mut self) -> Option<(bool, u32)> {
        self.scanner.skip_ascii_whitespace();
        let Some(byte) = self.scanner.peek_byte() else {
            // Source truncated inside the opening tag (`<div class="x"` at
            // EOF). Upstream fires `unexpected_eof` here; silently
            // propagating `None` would make the element — and everything
            // it referenced — vanish with zero errors.
            self.errors.push(ParseError::UnexpectedEof {
                range: Range::new(self.scanner.pos(), self.scanner.pos()),
            });
            return None;
        };
        match byte {
            b'>' => {
                self.scanner.advance_byte();
                Some((false, self.scanner.pos()))
            }
            b'/' => {
                self.scanner.advance_byte();
                self.scanner.skip_ascii_whitespace();
                if self.scanner.peek_byte() == Some(b'>') {
                    self.scanner.advance_byte();
                    Some((true, self.scanner.pos()))
                } else {
                    self.errors.push(ParseError::MalformedOpenTag {
                        range: Range::new(self.scanner.pos(), self.scanner.pos()),
                    });
                    None
                }
            }
            _ => {
                self.errors.push(ParseError::MalformedOpenTag {
                    range: Range::new(self.scanner.pos(), self.scanner.pos()),
                });
                None
            }
        }
    }

    fn parse_children_until(
        &mut self,
        tag: &str,
        open_start: u32,
        open_end: u32,
    ) -> Option<Fragment> {
        let frame = if is_component_tag(tag) || tag.starts_with("svelte:") {
            OpenFrame::Component(SmolStr::from(tag))
        } else {
            OpenFrame::Element(SmolStr::from(tag))
        };
        self.open_elements.push(frame);
        let result = self.parse_children_until_inner(tag, open_start, open_end);
        self.open_elements.pop();
        result
    }

    fn parse_children_until_inner(
        &mut self,
        tag: &str,
        open_start: u32,
        open_end: u32,
    ) -> Option<Fragment> {
        let children_start = self.scanner.pos();
        // A terminator surfacing here is a stray `{/...}` / `{:...}` inside
        // an element with no open block (e.g. `<div>{:else}</div>`) —
        // upstream errors. Report and resume so the element's remaining
        // children and closing tag still parse.
        let mut children = Vec::new();
        loop {
            let (mut more, stop) = self.parse_fragment_until(Some(tag));
            children.append(&mut more);
            match stop {
                None => break,
                Some(term) => self.push_stray_terminator_error(&term),
            }
        }
        let children_end = self.scanner.pos();

        // Implicitly closed by a sibling opening tag or an ancestor's
        // closing tag (HTML closing_tag_omitted rules). There is no
        // closing tag of our own to consume and nothing to report — the
        // element ends at the unconsumed trigger, exactly where upstream
        // sets `parent.end = start`.
        if self.pending_auto_close {
            self.pending_auto_close = false;
            return Some(Fragment {
                nodes: children,
                range: Range::new(children_start, children_end),
            });
        }

        // Consume the closing tag if present.
        if self.scanner.starts_with("</") {
            let close_start = self.scanner.pos();
            self.scanner.advance(2);
            // Consume tag name (match exactly against `tag`).
            for exp in tag.bytes() {
                match self.scanner.peek_byte() {
                    Some(b) if b == exp => self.scanner.advance_byte(),
                    _ => {
                        self.errors.push(ParseError::MismatchedClosingTag {
                            expected: tag.to_string(),
                            range: Range::new(close_start, self.scanner.pos()),
                        });
                        return None;
                    }
                }
            }
            self.scanner.skip_ascii_whitespace();
            if self.scanner.peek_byte() == Some(b'>') {
                self.scanner.advance_byte();
            }
        } else {
            // Missing closing tag.
            self.errors.push(ParseError::UnterminatedElement {
                name: tag.to_string(),
                range: Range::new(open_start, open_end),
            });
        }

        Some(Fragment {
            nodes: children,
            range: Range::new(children_start, children_end),
        })
    }

    fn parse_raw_text_children_until(
        &mut self,
        tag: &str,
        open_start: u32,
        open_end: u32,
    ) -> Option<Fragment> {
        let children_start = self.scanner.pos();
        let Some(close_start) = self.find_raw_text_close_tag(tag) else {
            self.errors.push(ParseError::UnterminatedElement {
                name: tag.to_string(),
                range: Range::new(open_start, open_end),
            });
            self.scanner.set_pos(self.fragment_end);
            return Some(Fragment {
                nodes: Vec::new(),
                range: Range::new(children_start, self.fragment_end),
            });
        };

        // CSS is not Svelte template syntax. Keep the source range for
        // diagnostics/future linting, but emit no child nodes until a CSS
        // parser exists.
        self.scanner.set_pos(close_start);
        let children_end = close_start;

        if self.scanner.starts_with("</") {
            let close_start = self.scanner.pos();
            self.scanner.advance(2);
            for exp in tag.bytes() {
                match self.scanner.peek_byte() {
                    Some(b) if b.eq_ignore_ascii_case(&exp) => self.scanner.advance_byte(),
                    _ => {
                        self.errors.push(ParseError::MismatchedClosingTag {
                            expected: tag.to_string(),
                            range: Range::new(close_start, self.scanner.pos()),
                        });
                        return None;
                    }
                }
            }
            self.scanner.skip_ascii_whitespace();
            if self.scanner.peek_byte() == Some(b'>') {
                self.scanner.advance_byte();
            }
        }

        Some(Fragment {
            nodes: Vec::new(),
            range: Range::new(children_start, children_end),
        })
    }

    /// Locate the matching raw-text close tag without interpreting the body.
    /// `<style>` may contain arbitrary CSS braces that would be valid CSS but
    /// invalid Svelte template syntax.
    fn find_raw_text_close_tag(&self, tag: &str) -> Option<u32> {
        let bytes = self.scanner.source().as_bytes();
        let tag_bytes = tag.as_bytes();
        let mut i = self.scanner.pos() as usize;
        let end = self.fragment_end as usize;

        while i + 2 + tag_bytes.len() <= end {
            if bytes[i] == b'<' && bytes.get(i + 1) == Some(&b'/') {
                let name_start = i + 2;
                let name_end = name_start + tag_bytes.len();
                let name_matches = bytes[name_start..name_end]
                    .iter()
                    .zip(tag_bytes)
                    .all(|(a, b)| a.eq_ignore_ascii_case(b));
                let boundary_matches = bytes.get(name_end).is_none_or(|b| {
                    !b.is_ascii_alphanumeric() && !matches!(b, b'-' | b'_' | b'.' | b':')
                });
                if name_matches && boundary_matches {
                    return Some(i as u32);
                }
            }
            i += 1;
        }

        None
    }

    /// The tag name at the current `<`, if it plausibly starts an opening
    /// tag. Used only for the implicit-close lookahead; full validation
    /// happens in [`Self::parse_element_or_component`].
    fn peek_open_tag_name(&self) -> Option<&'src str> {
        self.peek_tag_name_at(self.scanner.pos() + 1)
    }

    /// The tag name after the current `</`.
    fn peek_closing_tag_name(&self) -> Option<&'src str> {
        self.peek_tag_name_at(self.scanner.pos() + 2)
    }

    fn peek_tag_name_at(&self, start: u32) -> Option<&'src str> {
        let src = self.scanner.source();
        let bytes = src.as_bytes();
        let first = *bytes.get(start as usize)?;
        if !(first.is_ascii_alphabetic() || first >= 0x80) {
            return None;
        }
        let mut i = start as usize;
        while let Some(&b) = bytes.get(i) {
            if b.is_ascii_alphanumeric() || b >= 0x80 || matches!(b, b':' | b'-' | b'_' | b'.') {
                i += 1;
            } else {
                break;
            }
        }
        src.get(start as usize..i)
    }

    /// Whether the opening tag at the scanner implicitly closes the
    /// element currently being parsed, per the HTML5 closing_tag_omitted
    /// table (`<li>` while inside `<li>`, `<div>` while inside `<p>`, …).
    /// The table only contains plain HTML names, so component/`svelte:*`
    /// parents never match — mirroring upstream's
    /// `parent.type === 'RegularElement'` gate.
    fn open_tag_implicitly_closes(&self, stop_element: &str) -> bool {
        self.peek_open_tag_name()
            .is_some_and(|next| closing_tag_omitted(stop_element, Some(next)))
    }

    /// Whether the closing tag at the scanner (`</name>`, already known
    /// not to match the current element) closes an open ancestor such
    /// that every intervening frame — including the element currently
    /// being parsed (the top frame) — is a plain HTML element. Mirrors
    /// upstream's close-tag walk: plain-element parents pop with only the
    /// element_implicitly_closed warning; reaching a component or block
    /// boundary first is the error path instead.
    fn closing_tag_matches_open_ancestor(&self) -> bool {
        let Some(name) = self.peek_closing_tag_name() else {
            return false;
        };
        let mut frames = self.open_elements.iter().rev();
        // Top frame is the element whose children we're parsing; only a
        // plain element can be implicitly closed.
        if !matches!(frames.next(), Some(OpenFrame::Element(_))) {
            return false;
        }
        for frame in frames {
            match frame {
                // A non-matching plain element would pop too (with its
                // own warning) — keep walking.
                OpenFrame::Element(n) => {
                    if n.as_str() == name {
                        return true;
                    }
                }
                OpenFrame::Component(n) => return n.as_str() == name,
                OpenFrame::Block => return false,
            }
        }
        false
    }

    fn peek_closing_tag_matches(&self, tag: &str) -> bool {
        // self.scanner is at `</`. Check if what follows is `tag` then a
        // non-ident char.
        let after_slash = self.scanner.pos() + 2;
        let bytes = self.scanner.source().as_bytes();
        let tag_bytes = tag.as_bytes();
        let end = after_slash as usize + tag_bytes.len();
        if end > bytes.len() {
            return false;
        }
        if &bytes[after_slash as usize..end] != tag_bytes {
            return false;
        }
        match bytes.get(end).copied() {
            None => true,
            Some(b) => !b.is_ascii_alphanumeric() && !matches!(b, b'-' | b'_' | b'.' | b':'),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_ok(src: &str) -> Fragment {
        let range = Range::new(0, src.len() as u32);
        let (frag, errors) = parse_template(src, range);
        assert!(errors.is_empty(), "expected no errors, got {errors:?}");
        frag
    }

    #[test]
    fn empty_fragment() {
        let frag = parse_ok("");
        assert!(frag.nodes.is_empty());
    }

    #[test]
    fn plain_text() {
        let src = "hello world";
        let frag = parse_ok(src);
        assert_eq!(frag.nodes.len(), 1);
        assert!(matches!(frag.nodes[0], Node::Text(_)));
        if let Node::Text(t) = &frag.nodes[0] {
            assert_eq!(t.range.slice(src), "hello world");
        }
    }

    #[test]
    fn mustache_interpolation() {
        let frag = parse_ok("{value}");
        assert_eq!(frag.nodes.len(), 1);
        let Node::Interpolation(i) = &frag.nodes[0] else {
            panic!("expected Interpolation, got {:?}", frag.nodes[0]);
        };
        // expression_range should cover just "value" (no braces).
        assert_eq!(i.expression_range.slice("{value}"), "value");
        assert_eq!(i.range.slice("{value}"), "{value}");
    }

    #[test]
    fn text_then_interpolation_then_text() {
        let frag = parse_ok("before {value} after");
        assert_eq!(frag.nodes.len(), 3);
        assert!(matches!(frag.nodes[0], Node::Text(_)));
        assert!(matches!(frag.nodes[1], Node::Interpolation(_)));
        assert!(matches!(frag.nodes[2], Node::Text(_)));
    }

    #[test]
    fn html_comment() {
        let src = "<!-- hi -->";
        let frag = parse_ok(src);
        assert_eq!(frag.nodes.len(), 1);
        let Node::Comment(c) = &frag.nodes[0] else {
            panic!("expected Comment");
        };
        assert_eq!(c.data_range.slice(src), " hi ");
    }

    #[test]
    fn nested_style_body_is_opaque() {
        let src = r#"<svelte:head>
  <style>
    @keyframes fadeIn {
      from {
        opacity: 0;
      }
      to {
        opacity: 1;
      }
    }
  </style>
</svelte:head>"#;
        let frag = parse_ok(src);
        let [Node::SvelteElement(head)] = frag.nodes.as_slice() else {
            panic!("expected one svelte:head node, got {:?}", frag.nodes);
        };
        let style = head
            .children
            .nodes
            .iter()
            .find_map(|node| match node {
                Node::Element(element) if element.name.as_str() == "style" => Some(element),
                _ => None,
            })
            .expect("expected style child");

        assert_eq!(style.name.as_str(), "style");
        assert!(
            style.children.nodes.is_empty(),
            "style contents are CSS, not Svelte template expressions"
        );
    }

    #[test]
    fn comment_with_dashes_inside() {
        let frag = parse_ok("<!-- a-b -->");
        assert_eq!(frag.nodes.len(), 1);
    }

    #[test]
    fn simple_element() {
        let frag = parse_ok("<div></div>");
        assert_eq!(frag.nodes.len(), 1);
        let Node::Element(e) = &frag.nodes[0] else {
            panic!("expected Element");
        };
        assert_eq!(e.name, "div");
        assert!(e.children.nodes.is_empty());
        assert!(!e.self_closing);
    }

    #[test]
    fn element_with_text_child() {
        let frag = parse_ok("<p>hello</p>");
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.children.nodes.len(), 1);
        assert!(matches!(e.children.nodes[0], Node::Text(_)));
    }

    #[test]
    fn element_with_mixed_children() {
        let frag = parse_ok("<p>hello {name}!</p>");
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.children.nodes.len(), 3);
    }

    #[test]
    fn nested_elements() {
        let frag = parse_ok("<div><span>inner</span></div>");
        let Node::Element(outer) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(outer.name, "div");
        assert_eq!(outer.children.nodes.len(), 1);
        let Node::Element(inner) = &outer.children.nodes[0] else {
            unreachable!()
        };
        assert_eq!(inner.name, "span");
    }

    #[test]
    fn self_closing_element() {
        let frag = parse_ok("<br />");
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.name, "br");
        assert!(e.self_closing);
    }

    // ===== HTML implicit-close (upstream element.js closing_tag_omitted) =====

    #[test]
    fn li_implicitly_closed_by_sibling_li() {
        // `<ul><li>a<li>b</ul>` is valid Svelte: upstream closes the first
        // `<li>` at the second `<li>` per closing_tag_omitted and emits
        // only the element_implicitly_closed WARNING.
        let src = "<ul><li>a<li>b</ul>";
        let frag = parse_ok(src);
        let [Node::Element(ul)] = frag.nodes.as_slice() else {
            panic!("expected one ul, got {:?}", frag.nodes);
        };
        assert_eq!(ul.name, "ul");
        assert_eq!(ul.range.slice(src), src);
        let lis: Vec<&Element> = ul
            .children
            .nodes
            .iter()
            .filter_map(|n| match n {
                Node::Element(e) if e.name == "li" => Some(e),
                _ => None,
            })
            .collect();
        assert_eq!(lis.len(), 2, "two li siblings, got {:?}", ul.children.nodes);
        // The auto-closed element ends where the closing trigger starts
        // (upstream sets parent.end = start of the new tag / close tag).
        assert_eq!(lis[0].range.slice(src), "<li>a");
        assert_eq!(lis[1].range.slice(src), "<li>b");
    }

    #[test]
    fn p_implicitly_closed_by_ancestor_close_tag() {
        // `<div><p>text</div>` — the `</div>` implicitly closes the open
        // `<p>` (upstream pops RegularElement parents with a warning).
        let src = "<div><p>text</div>";
        let frag = parse_ok(src);
        let [Node::Element(div)] = frag.nodes.as_slice() else {
            panic!("expected one div, got {:?}", frag.nodes);
        };
        assert_eq!(div.range.slice(src), src);
        let [Node::Element(p)] = div.children.nodes.as_slice() else {
            panic!("expected one p child, got {:?}", div.children.nodes);
        };
        assert_eq!(p.name, "p");
        assert_eq!(p.range.slice(src), "<p>text");
    }

    #[test]
    fn ancestor_close_pops_multiple_open_elements() {
        // `<ul><li><b>x</ul>` — upstream pops BOTH `<b>` and `<li>` with
        // warnings when `</ul>` arrives (any RegularElement pops, not just
        // table entries).
        let src = "<ul><li><b>x</ul>";
        let frag = parse_ok(src);
        let [Node::Element(ul)] = frag.nodes.as_slice() else {
            panic!("expected one ul, got {:?}", frag.nodes);
        };
        let [Node::Element(li)] = ul.children.nodes.as_slice() else {
            panic!("expected one li child, got {:?}", ul.children.nodes);
        };
        let [Node::Element(b)] = li.children.nodes.as_slice() else {
            panic!("expected one b child, got {:?}", li.children.nodes);
        };
        assert_eq!(b.range.slice(src), "<b>x");
    }

    #[test]
    fn li_implicitly_closed_by_component_close_tag() {
        // `<Comp><li>x</Comp>` parses clean upstream: the `</Comp>` pops
        // the open `<li>` with a warning and then closes the component.
        let src = "<Comp><li>x</Comp>";
        let frag = parse_ok(src);
        let [Node::Component(c)] = frag.nodes.as_slice() else {
            panic!("expected one component, got {:?}", frag.nodes);
        };
        assert_eq!(c.range.slice(src), src);
        let [Node::Element(li)] = c.children.nodes.as_slice() else {
            panic!("expected one li child, got {:?}", c.children.nodes);
        };
        assert_eq!(li.range.slice(src), "<li>x");
    }

    #[test]
    fn component_is_never_implicitly_closed() {
        // `<div><Comp></div>` errors upstream
        // (element_invalid_closing_tag: Comp isn't a RegularElement).
        let src = "<div><Comp></div>";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(!errors.is_empty(), "expected errors for {src:?}");
    }

    #[test]
    fn implicit_close_does_not_cross_block_boundary() {
        // `<div>{#if c}<li></div>{/if}` errors upstream — the ancestor
        // walk stops at the IfBlock (not a RegularElement).
        let src = "<div>{#if c}<li></div>{/if}";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(!errors.is_empty(), "expected errors for {src:?}");
    }

    #[test]
    fn open_tag_auto_close_table_variants() {
        // A sampling of closing_tag_omitted rows beyond li: options,
        // table cells/rows, dt/dd, and p closed by a block-level element.
        for (src, outer, inner, count) in [
            ("<select><option>a<option>b</select>", "select", "option", 2),
            ("<tr><td>a<td>b<td>c</tr>", "tr", "td", 3),
            ("<dl><dt>t<dd>d</dl>", "dl", "dt", 1),
        ] {
            let frag = parse_ok(src);
            let [Node::Element(o)] = frag.nodes.as_slice() else {
                panic!("expected one {outer}, got {:?}", frag.nodes);
            };
            assert_eq!(o.name, outer);
            let inners = o
                .children
                .nodes
                .iter()
                .filter(|n| matches!(n, Node::Element(e) if e.name == inner))
                .count();
            assert_eq!(
                inners, count,
                "{count} {inner} in {src:?}: {:?}",
                o.children.nodes
            );
        }
        // `<p>` is implicitly closed by a following block-level element.
        let src = "<p>a<div>b</div>";
        let frag = parse_ok(src);
        assert_eq!(frag.nodes.len(), 2, "p and div siblings: {:?}", frag.nodes);
        let Node::Element(p) = &frag.nodes[0] else {
            panic!("expected p first");
        };
        assert_eq!(p.range.slice(src), "<p>a");
    }

    #[test]
    fn doctype_parses_as_void_element() {
        // `<!DOCTYPE html>` is a valid Svelte template node — upstream's
        // is_valid_element_name admits /^![a-zA-Z]+$/ and is_void treats
        // `!doctype` (any case) as void, so no closing tag is expected
        // and the rest of the file parses normally.
        for (src, name) in [
            ("<!DOCTYPE html><div>x</div>", "!DOCTYPE"),
            ("<!doctype html><div>x</div>", "!doctype"),
        ] {
            let (frag, errors) = parse_template(src, Range::new(0, src.len() as u32));
            assert!(
                errors.is_empty(),
                "expected no errors for {src:?}: {errors:?}"
            );
            assert_eq!(frag.nodes.len(), 2, "two nodes for {src:?}");
            let Node::Element(doctype) = &frag.nodes[0] else {
                panic!(
                    "expected doctype element for {src:?}, got {:?}",
                    frag.nodes[0]
                );
            };
            assert_eq!(doctype.name, name);
            assert!(doctype.self_closing, "doctype is void");
            assert!(doctype.children.nodes.is_empty());
            let Node::Element(div) = &frag.nodes[1] else {
                panic!("expected div for {src:?}, got {:?}", frag.nodes[1]);
            };
            assert_eq!(div.name, "div");
        }
    }

    #[test]
    fn non_doctype_bang_tag_still_requires_close() {
        // `<!foo>` is a valid element NAME upstream but not void — only
        // `!doctype` is. Left unclosed it errors, same as upstream's
        // element_unclosed.
        let src = "<!foo>x";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::UnterminatedElement { .. })),
            "expected UnterminatedElement, got {errors:?}"
        );
    }

    #[test]
    fn bang_without_letter_is_still_malformed() {
        let src = "<![CDATA[x]]>";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::MalformedOpenTag { .. })),
            "expected MalformedOpenTag, got {errors:?}"
        );
    }

    #[test]
    fn void_element_without_closing() {
        // <br> (no `/`) is a void element; should parse as self-closing
        // without requiring </br>.
        let frag = parse_ok("<br>after");
        assert_eq!(frag.nodes.len(), 2);
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.name, "br");
        assert!(e.self_closing);
    }

    #[test]
    fn component_by_uppercase_first_letter() {
        let frag = parse_ok("<Button>click</Button>");
        assert_eq!(frag.nodes.len(), 1);
        let Node::Component(c) = &frag.nodes[0] else {
            panic!("expected Component, got {:?}", frag.nodes[0]);
        };
        assert_eq!(c.name, "Button");
    }

    #[test]
    fn component_with_dotted_name() {
        let frag = parse_ok("<ui.Button></ui.Button>");
        let Node::Component(c) = &frag.nodes[0] else {
            panic!("expected Component");
        };
        assert_eq!(c.name, "ui.Button");
    }

    #[test]
    fn custom_element_is_not_component() {
        // my-widget has a hyphen; lowercased first → treat as HTML element.
        let frag = parse_ok("<my-widget></my-widget>");
        assert!(matches!(frag.nodes[0], Node::Element(_)));
    }

    #[test]
    fn svelte_special_element() {
        let frag = parse_ok("<svelte:self />");
        let Node::SvelteElement(s) = &frag.nodes[0] else {
            panic!("expected SvelteElement");
        };
        assert_eq!(s.kind, SvelteElementKind::SelfRef);
        assert!(s.self_closing);
    }

    #[test]
    fn svelte_window() {
        let frag = parse_ok("<svelte:window />");
        let Node::SvelteElement(s) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(s.kind, SvelteElementKind::Window);
    }

    #[test]
    fn unknown_svelte_colon_errors() {
        let src = "<svelte:notAThing />";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::UnknownSvelteElement { .. }))
        );
    }

    #[test]
    fn interpolation_with_object_literal() {
        let frag = parse_ok("<p>{{ key: 'value' }}</p>");
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.children.nodes.len(), 1);
        assert!(matches!(e.children.nodes[0], Node::Interpolation(_)));
    }

    #[test]
    fn interpolation_with_template_literal() {
        let frag = parse_ok("<p>{`hello ${name}!`}</p>");
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        let Node::Interpolation(i) = &e.children.nodes[0] else {
            panic!("expected interpolation");
        };
        assert!(
            i.expression_range
                .slice("<p>{`hello ${name}!`}</p>")
                .contains("`hello ${name}!`")
        );
    }

    #[test]
    fn stray_top_level_block_close_errors_and_parsing_resumes() {
        // Upstream fires block_unexpected_close for `{/if}` with no open
        // block. Previously the terminator was silently consumed and
        // parsing STOPPED — everything after it was dropped with zero
        // errors.
        let src = "{/if}<div>{undeclared}</div>";
        let (frag, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::UnexpectedBlockClose { .. })),
            "expected UnexpectedBlockClose, got {errors:?}"
        );
        let err = errors
            .iter()
            .find(|e| matches!(e, ParseError::UnexpectedBlockClose { .. }))
            .unwrap();
        assert_eq!(err.range().slice(src), "{/if}");
        // The rest of the template must still be parsed.
        let Some(Node::Element(e)) = frag.nodes.first() else {
            panic!("expected the div to survive, got {:?}", frag.nodes);
        };
        assert_eq!(e.name, "div");
        assert!(matches!(e.children.nodes[0], Node::Interpolation(_)));
    }

    #[test]
    fn stray_top_level_continuation_errors_and_parsing_resumes() {
        // Upstream fires block_invalid_continuation_placement for a
        // `{:...}` tag with no open block.
        let src = "{:else}<p>after</p>";
        let (frag, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::InvalidBlockContinuation { .. })),
            "expected InvalidBlockContinuation, got {errors:?}"
        );
        assert!(
            frag.nodes
                .iter()
                .any(|n| matches!(n, Node::Element(e) if e.name == "p")),
            "expected the p to survive, got {:?}",
            frag.nodes
        );
    }

    #[test]
    fn stray_continuation_inside_element_errors() {
        // `<div>{:else}</div>` — upstream errors
        // block_invalid_continuation_placement. Previously the `{:else}`
        // was swallowed with zero errors.
        let src = "<div>{:else}</div>";
        let (frag, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::InvalidBlockContinuation { .. })),
            "expected InvalidBlockContinuation, got {errors:?}"
        );
        // The element itself still parses and consumes its closing tag.
        let Some(Node::Element(e)) = frag.nodes.first() else {
            panic!("expected the div node, got {:?}", frag.nodes);
        };
        assert_eq!(e.name, "div");
        assert_eq!(e.range.slice(src), src);
    }

    #[test]
    fn stray_block_close_inside_element_errors() {
        let src = "<div>{/if}</div>";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::UnexpectedBlockClose { .. })),
            "expected UnexpectedBlockClose, got {errors:?}"
        );
    }

    #[test]
    fn eof_inside_opening_tag_errors() {
        // A file truncated inside an opening tag must report an error —
        // upstream fires `unexpected_eof`. Previously the element (and
        // everything it referenced) vanished with zero errors and the
        // file "checked clean".
        for src in ["<div class=\"x\"", "<div", "<div class=\"x\" id=\"y\""] {
            let (frag, errors) = parse_template(src, Range::new(0, src.len() as u32));
            assert!(
                errors
                    .iter()
                    .any(|e| matches!(e, ParseError::UnexpectedEof { .. })),
                "expected UnexpectedEof for {src:?}, got {errors:?} (nodes: {:?})",
                frag.nodes
            );
        }
    }

    #[test]
    fn mismatched_closing_tag_errors() {
        let src = "<div><span></div></span>";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(!errors.is_empty());
    }

    #[test]
    fn if_block_parses() {
        let frag = parse_ok("{#if cond}<p>yes</p>{/if}");
        assert_eq!(frag.nodes.len(), 1);
        let Node::IfBlock(b) = &frag.nodes[0] else {
            panic!("expected IfBlock, got {:?}", frag.nodes[0]);
        };
        assert_eq!(
            b.condition_range.slice("{#if cond}<p>yes</p>{/if}").trim(),
            "cond"
        );
        assert_eq!(b.consequent.nodes.len(), 1); // <p>
    }

    #[test]
    fn if_else_block_parses() {
        let frag = parse_ok("{#if cond}A{:else}B{/if}");
        let Node::IfBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.alternate.is_some());
        assert_eq!(b.elseif_arms.len(), 0);
    }

    #[test]
    fn if_elseif_else_block_parses() {
        let frag = parse_ok("{#if a}A{:else if b}B{:else if c}C{:else}D{/if}");
        let Node::IfBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(b.elseif_arms.len(), 2);
        assert!(b.alternate.is_some());
    }

    #[test]
    fn each_block_parses() {
        let frag = parse_ok("{#each items as item}<li>{item}</li>{/each}");
        let Node::EachBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        let src = "{#each items as item}<li>{item}</li>{/each}";
        assert_eq!(b.expression_range.slice(src).trim(), "items");
        let clause = b.as_clause.as_ref().unwrap();
        assert_eq!(clause.context_range.slice(src), "item");
    }

    #[test]
    fn each_block_with_index_and_key() {
        let src = "{#each items as item, i (item.id)}<li>{item}</li>{/each}";
        let frag = parse_ok(src);
        let Node::EachBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        let clause = b.as_clause.as_ref().unwrap();
        assert_eq!(clause.context_range.slice(src), "item");
        assert_eq!(clause.index_range.map(|r| r.slice(src)), Some("i"));
        assert_eq!(clause.key_range.map(|r| r.slice(src)), Some("item.id"));
    }

    #[test]
    fn each_block_with_newline_before_as() {
        let src = "{#each items\nas item}<b>{item}</b>{/each}";
        let frag = parse_ok(src);
        let Node::EachBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(b.expression_range.slice(src), "items");
        let clause = b.as_clause.as_ref().expect("as-clause parsed");
        assert_eq!(clause.context_range.slice(src), "item");
    }

    #[test]
    fn each_block_without_as_clause() {
        // Svelte allows `{#each items}` with no binding (iterate N times,
        // discard the item). Parser must distinguish this from the more
        // common `{#each items as item}` form.
        let frag = parse_ok("{#each items}<span>x</span>{/each}");
        let Node::EachBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.as_clause.is_none());
    }

    #[test]
    fn each_with_else() {
        let frag = parse_ok("{#each items as item}A{:else}empty{/each}");
        let Node::EachBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.alternate.is_some());
    }

    #[test]
    fn await_block_full_form() {
        let frag = parse_ok("{#await p}loading{:then v}{v}{:catch e}err{/await}");
        let Node::AwaitBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.pending.is_some());
        assert!(b.then_branch.is_some());
        assert!(b.catch_branch.is_some());
    }

    #[test]
    fn await_block_short_then_form() {
        let frag = parse_ok("{#await p then v}{v}{/await}");
        let Node::AwaitBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.pending.is_none());
        assert!(b.then_branch.is_some());
    }

    #[test]
    fn await_block_newline_before_then() {
        let src = "{#await p\nthen v}{v}{/await}";
        let frag = parse_ok(src);
        let Node::AwaitBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.pending.is_none());
        assert_eq!(b.expression_range.slice(src), "p");
        let then = b.then_branch.as_ref().expect("then branch parsed");
        assert_eq!(then.context_range.map(|r| r.slice(src)), Some("v"));
    }

    #[test]
    fn await_block_tab_before_catch() {
        let src = "{#await p\tcatch e}{e}{/await}";
        let frag = parse_ok(src);
        let Node::AwaitBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        let catch = b.catch_branch.as_ref().expect("catch branch parsed");
        assert_eq!(catch.context_range.map(|r| r.slice(src)), Some("e"));
    }

    #[test]
    fn await_short_then_with_trailing_catch() {
        // `{#await p then v} body {:catch} fallback {/await}` — Svelte
        // grammar allows `:catch` after a then-short-form. Pre-fix this
        // tripped UnterminatedElement{await} because the parser bailed
        // at `{:catch}` expecting `{/await}`.
        let frag = parse_ok("{#await p then v}<b/>{:catch}<c/>{/await}");
        let Node::AwaitBlock(b) = &frag.nodes[0] else {
            unreachable!()
        };
        assert!(b.then_branch.is_some());
        assert!(b.catch_branch.is_some());
    }

    #[test]
    fn key_block_parses() {
        let frag = parse_ok("{#key trigger}<x />{/key}");
        let Node::KeyBlock(b) = &frag.nodes[0] else {
            panic!("expected KeyBlock");
        };
        assert_eq!(
            b.expression_range.slice("{#key trigger}<x />{/key}").trim(),
            "trigger"
        );
    }

    #[test]
    fn snippet_block_parses() {
        let src = "{#snippet row(item, i)}<tr>{i}: {item.name}</tr>{/snippet}";
        let frag = parse_ok(src);
        let Node::SnippetBlock(b) = &frag.nodes[0] else {
            panic!("expected SnippetBlock");
        };
        assert_eq!(b.name, "row");
        assert_eq!(b.parameters_range.slice(src), "item, i");
    }

    #[test]
    fn unknown_block_tag_records_error_and_skips() {
        let src = "{#deferred x}body{/deferred}";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::UnsupportedBlock { .. }))
        );
    }

    #[test]
    fn at_tags_modeled_as_interpolations() {
        // `{@html}`, `{@render}`, `{@const}`, `{@debug}`, `{@attach}` —
        // each is parsed as an Interpolation whose expression range
        // covers the body after the tag keyword. This is enough for the
        // template-ref pass to extract identifier references, which is
        // all type-checking needs at this level. Real per-tag semantics
        // (e.g. `@const` introduces a binding) aren't modeled.
        for src in ["{@html foo}", "{@render fn(arg)}", "{@debug x, y}"] {
            let (frag, errors) = parse_template(src, Range::new(0, src.len() as u32));
            assert!(errors.is_empty(), "no errors for {src}: {errors:?}");
            assert_eq!(frag.nodes.len(), 1, "single node for {src}");
            assert!(
                matches!(frag.nodes[0], Node::Interpolation(_)),
                "interpolation for {src}, got {:?}",
                frag.nodes[0]
            );
        }
    }

    #[test]
    fn element_with_simple_attrs_parsed() {
        let frag = parse_ok(r#"<div class="foo" id="bar">body</div>"#);
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.name, "div");
        assert_eq!(e.attributes.len(), 2);
        assert_eq!(e.children.nodes.len(), 1);
    }

    #[test]
    fn element_with_expression_attr_skipped_cleanly() {
        let frag = parse_ok(r#"<button onclick={handler}>click</button>"#);
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.name, "button");
    }

    #[test]
    fn parse_all_template_runs_merges_runs() {
        let src = "before<script>x</script>middle<style>y</style>after";
        // Simulate the runs that sections parser would emit.
        let runs = &[Range::new(0, 6), Range::new(24, 30), Range::new(45, 50)];
        let (frag, errors) = parse_all_template_runs(src, runs);
        assert!(errors.is_empty(), "{errors:?}");
        // 3 text nodes.
        assert_eq!(frag.nodes.len(), 3);
    }

    #[test]
    fn ranges_are_absolute_offsets_into_original_source() {
        // A fragment in the middle of a source file should produce ranges
        // that point at the original offsets, not re-based to 0.
        let src = "<script>x</script><p>hi</p>";
        let run_range = Range::new(18, src.len() as u32);
        let (frag, errors) = parse_template(src, run_range);
        assert!(errors.is_empty(), "{errors:?}");
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.range.start, 18);
        assert_eq!(e.range.slice(src), "<p>hi</p>");
    }
}
