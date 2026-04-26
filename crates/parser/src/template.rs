//! Template-level parser.
//!
//! Parses the template fragments recovered by the structural section parser
//! into a proper [`Fragment`] AST. Current scope:
//!
//! - Text
//! - `{expression}` mustache interpolations
//! - `<!-- comments -->`
//! - Elements / components / `<svelte:*>` **without attributes** (attributes
//!   land in a subsequent commit)
//! - Void + self-closing elements
//!
//! Control-flow blocks, tags (`{@html}`/`{@const}`/`{@render}`/`{@debug}`/`{@attach}`),
//! attributes and directives are **not yet implemented**. Encountering any of
//! those currently produces a `NotYetImplemented` error with the offending
//! range; upgrade as we land each grammar piece.

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
use crate::mustache::find_mustache_end;
use crate::scanner::Scanner;

/// Parse one template fragment out of the source string.
///
/// `range` is the byte range within `source` that contains template text.
/// The returned [`Fragment`] has `range == range`.
pub fn parse_template(source: &str, range: Range) -> (Fragment, Vec<ParseError>) {
    let mut parser = TemplateParser::new(source, range);
    let (nodes, _stop) = parser.parse_fragment_until(None);
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

struct TemplateParser<'src> {
    scanner: Scanner<'src>,
    fragment_range: Range,
    fragment_end: u32,
    errors: Vec<ParseError>,
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
        }
    }

    fn at_fragment_end(&self) -> bool {
        self.scanner.pos() >= self.fragment_end || self.scanner.eof()
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
                if let Some(term) = peek_and_consume_terminator(&mut self.scanner, &mut self.errors)
                {
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
                match find_mustache_end(self.scanner.source(), self.scanner.pos() + 1) {
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

            // `</TAG>` closing tag.
            if self.scanner.starts_with("</") {
                if let Some(tag) = stop_element {
                    if self.peek_closing_tag_matches(tag) {
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

        match name {
            "if" => self.parse_if_block(block_start),
            "each" => self.parse_each_block(block_start),
            "await" => self.parse_await_block(block_start),
            "key" => self.parse_key_block(block_start),
            "snippet" => self.parse_snippet_block(block_start),
            other => {
                self.errors.push(ParseError::UnsupportedBlock {
                    range: Range::new(block_start, self.scanner.pos()),
                });
                // Skip to the end of this opening mustache and beyond to the
                // matching `{/other}` if we can find one — else bail.
                if let Some(end) = find_mustache_end(self.scanner.source(), self.scanner.pos()) {
                    self.scanner.set_pos(end + 1);
                }
                let needle = format!("{{/{other}}}");
                if let Some(pos) = self.scanner.find(needle.as_bytes()) {
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
                    let (body_nodes, t) = self.parse_fragment_until(None);
                    elseif_arms.push(ElseIfArm {
                        condition_range,
                        body: Fragment {
                            nodes: body_nodes,
                            range: Range::new(0, 0),
                        },
                    });
                    next = t;
                }
                Some(BlockTerminator::Else) => {
                    let (body_nodes, t) = self.parse_fragment_until(None);
                    alternate = Some(Fragment {
                        nodes: body_nodes,
                        range: Range::new(0, 0),
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
                let (alt_nodes, t2) = self.parse_fragment_until(None);
                alternate = Some(Fragment {
                    nodes: alt_nodes,
                    range: Range::new(0, 0),
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
                let (body, mut term) = self.parse_fragment_until(None);
                then_branch = Some(ThenBranch {
                    context_range: ctx,
                    body: Fragment {
                        nodes: body,
                        range: Range::new(0, 0),
                    },
                });
                if let Some(BlockTerminator::Catch { context_range }) = &term {
                    let cctx = *context_range;
                    let (catch_nodes, t2) = self.parse_fragment_until(None);
                    catch_branch = Some(CatchBranch {
                        context_range: cctx,
                        body: Fragment {
                            nodes: catch_nodes,
                            range: Range::new(0, 0),
                        },
                    });
                    term = t2;
                }
                self.finish_await_block(term, block_start);
            }
            AwaitShortForm::Catch(ctx) => {
                let (body, t) = self.parse_fragment_until(None);
                catch_branch = Some(CatchBranch {
                    context_range: ctx,
                    body: Fragment {
                        nodes: body,
                        range: Range::new(0, 0),
                    },
                });
                self.finish_await_block(t, block_start);
            }
            AwaitShortForm::None => {
                // Full form: pending, then optional :then, optional :catch, {/await}.
                let (pending_nodes, mut term) = self.parse_fragment_until(None);
                pending = Some(Fragment {
                    nodes: pending_nodes,
                    range: Range::new(0, 0),
                });
                // :then?
                if let Some(BlockTerminator::Then { context_range }) = &term {
                    let ctx = *context_range;
                    let (then_nodes, t2) = self.parse_fragment_until(None);
                    then_branch = Some(ThenBranch {
                        context_range: ctx,
                        body: Fragment {
                            nodes: then_nodes,
                            range: Range::new(0, 0),
                        },
                    });
                    term = t2;
                }
                // :catch?
                if let Some(BlockTerminator::Catch { context_range }) = &term {
                    let ctx = *context_range;
                    let (catch_nodes, t2) = self.parse_fragment_until(None);
                    catch_branch = Some(CatchBranch {
                        context_range: ctx,
                        body: Fragment {
                            nodes: catch_nodes,
                            range: Range::new(0, 0),
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
        let content = self.scanner.source()[start as usize..end as usize].to_string();
        Node::Text(Text {
            content,
            range: Range::new(start, end),
        })
    }

    fn parse_comment(&mut self) -> Node {
        let start = self.scanner.pos();
        debug_assert!(self.scanner.starts_with("<!--"));
        self.scanner.advance(4);
        let body_start = self.scanner.pos();

        let body_end = match self.scanner.find(b"-->") {
            Some(pos) => pos,
            None => {
                // Unterminated — treat everything to fragment end as the body.
                let end = self.fragment_end;
                self.errors.push(ParseError::UnterminatedComment {
                    range: Range::new(start, end),
                });
                self.scanner.set_pos(end);
                let data = self.scanner.source()[body_start as usize..end as usize].to_string();
                return Node::Comment(Comment {
                    data,
                    range: Range::new(start, end),
                });
            }
        };

        let data = self.scanner.source()[body_start as usize..body_end as usize].to_string();
        self.scanner.set_pos(body_end);
        self.scanner.advance(3); // past "-->"
        Node::Comment(Comment {
            data,
            range: Range::new(start, self.scanner.pos()),
        })
    }

    fn parse_interpolation(&mut self) -> Option<Node> {
        let start = self.scanner.pos();
        debug_assert_eq!(self.scanner.peek_byte(), Some(b'{'));
        self.scanner.advance_byte();
        let expr_start = self.scanner.pos();

        match find_mustache_end(self.scanner.source(), expr_start) {
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
        if !self
            .scanner
            .peek_byte()
            .map(|b| b.is_ascii_alphabetic())
            .unwrap_or(false)
        {
            // Not a valid element start — record error, skip `<`.
            self.errors.push(ParseError::MalformedOpenTag {
                range: Range::new(tag_start, self.scanner.pos()),
            });
            return None;
        }

        while let Some(b) = self.scanner.peek_byte() {
            if b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_' | b'.') {
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
        match self.scanner.peek_byte()? {
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
        let children_start = self.scanner.pos();
        let (children, _stop) = self.parse_fragment_until(Some(tag));
        let children_end = self.scanner.pos();

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
        let frag = parse_ok("hello world");
        assert_eq!(frag.nodes.len(), 1);
        assert!(matches!(frag.nodes[0], Node::Text(_)));
        if let Node::Text(t) = &frag.nodes[0] {
            assert_eq!(t.content, "hello world");
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
        let frag = parse_ok("<!-- hi -->");
        assert_eq!(frag.nodes.len(), 1);
        let Node::Comment(c) = &frag.nodes[0] else {
            panic!("expected Comment");
        };
        assert_eq!(c.data, " hi ");
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
    fn element_with_simple_attrs_skipped_cleanly() {
        // Attributes are not yet supported — we only verify the parser
        // doesn't blow up on them.
        let frag = parse_ok(r#"<div class="foo" id="bar">body</div>"#);
        let Node::Element(e) = &frag.nodes[0] else {
            unreachable!()
        };
        assert_eq!(e.name, "div");
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
