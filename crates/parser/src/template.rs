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
    Comment, Component, Element, Fragment, Interpolation, Node, SvelteElement, SvelteElementKind,
    Text, is_component_tag, is_void_element,
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
    let nodes = parser.parse_fragment_until(None);
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

    /// Parse a sequence of nodes until either the fragment end is reached or
    /// a closing tag matching `stop_tag` is encountered. Returns the parsed
    /// nodes; on a stop-tag match, leaves the scanner positioned at the
    /// start of that closing tag so the caller can consume it.
    fn parse_fragment_until(&mut self, stop_tag: Option<&str>) -> Vec<Node> {
        let mut nodes = Vec::new();
        while !self.at_fragment_end() {
            // Detect closing tag: `</TAG` or `</`.
            if self.scanner.starts_with("</") {
                if let Some(tag) = stop_tag {
                    // Peek whether this closing tag matches `tag`.
                    if self.peek_closing_tag_matches(tag) {
                        return nodes;
                    }
                }
                // Unmatched closing tag inside a fragment — record error and
                // advance past `<` to avoid infinite loop.
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
                match self.parse_element_or_component() {
                    Some(node) => nodes.push(node),
                    None => {
                        // parse_element_or_component already reported the error
                        // and advanced past `<`; continue.
                    }
                }
                continue;
            }

            if self.scanner.peek_byte() == Some(b'{') {
                // Block expressions ({#if}, {#each}, {:else}, {/if}, {@html}, etc.)
                // are not yet supported. Distinguish from a plain
                // interpolation by looking at the char after `{`.
                match self.scanner.peek_byte_at(1) {
                    Some(b'#') | Some(b':') | Some(b'/') | Some(b'@') => {
                        let start = self.scanner.pos();
                        // Skip the opening `{` and the rest up to the matching `}`
                        // so we don't loop forever.
                        match find_mustache_end(self.scanner.source(), self.scanner.pos() + 1) {
                            Some(end) => {
                                self.errors.push(ParseError::UnsupportedBlock {
                                    range: Range::new(start, end + 1),
                                });
                                self.scanner.set_pos(end + 1);
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
                    _ => {}
                }
                match self.parse_interpolation() {
                    Some(node) => nodes.push(node),
                    None => {
                        // Error already recorded; advance one byte.
                        self.scanner.advance_byte();
                    }
                }
                continue;
            }

            nodes.push(self.parse_text());
        }
        nodes
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

        // Attributes — SKIPPED in this commit. We scan until `>` or `/>`,
        // ignoring everything (including values with `>` inside quotes, which
        // is handled crudely but correctly for well-formed input).
        let (self_closing, open_tag_end) = self.skip_until_tag_close()?;

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
            children,
            self_closing: self_closing || is_void,
            range: Range::new(tag_start, self.scanner.pos()),
        }))
    }

    /// Advance past attributes and the `>` or `/>` of an opening tag.
    /// Returns `(self_closing, end_pos)` where `end_pos` is just past the
    /// closing delimiter. On EOF returns `None`.
    fn skip_until_tag_close(&mut self) -> Option<(bool, u32)> {
        while !self.at_fragment_end() {
            match self.scanner.peek_byte()? {
                b'>' => {
                    self.scanner.advance_byte();
                    return Some((false, self.scanner.pos()));
                }
                b'/' => {
                    self.scanner.advance_byte();
                    if self.scanner.peek_byte() == Some(b'>') {
                        self.scanner.advance_byte();
                        return Some((true, self.scanner.pos()));
                    }
                }
                b'"' => {
                    // Skip quoted attribute value.
                    self.scanner.advance_byte();
                    while let Some(b) = self.scanner.peek_byte() {
                        self.scanner.advance_byte();
                        if b == b'"' {
                            break;
                        }
                    }
                }
                b'\'' => {
                    self.scanner.advance_byte();
                    while let Some(b) = self.scanner.peek_byte() {
                        self.scanner.advance_byte();
                        if b == b'\'' {
                            break;
                        }
                    }
                }
                b'{' => {
                    // Expression attribute like `{name}` or `={expr}` or spread.
                    // Skip the whole mustache block.
                    let open = self.scanner.pos();
                    self.scanner.advance_byte();
                    match find_mustache_end(self.scanner.source(), self.scanner.pos()) {
                        Some(end) => self.scanner.set_pos(end + 1),
                        None => {
                            self.errors.push(ParseError::UnterminatedMustache {
                                range: Range::new(open, self.fragment_end),
                            });
                            self.scanner.set_pos(self.fragment_end);
                            return None;
                        }
                    }
                }
                _ => {
                    self.scanner.advance_char();
                }
            }
        }
        self.errors.push(ParseError::MalformedOpenTag {
            range: Range::new(self.fragment_end, self.fragment_end),
        });
        None
    }

    fn parse_children_until(
        &mut self,
        tag: &str,
        open_start: u32,
        open_end: u32,
    ) -> Option<Fragment> {
        let children_start = self.scanner.pos();
        let children = self.parse_fragment_until(Some(tag));
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
    fn unsupported_block_tag_records_error_but_doesnt_hang() {
        let src = "{#if condition}<p>yes</p>{/if}";
        let (_, errors) = parse_template(src, Range::new(0, src.len() as u32));
        // {#if} and {/if} both recorded as unsupported.
        assert!(
            errors
                .iter()
                .any(|e| matches!(e, ParseError::UnsupportedBlock { .. })),
            "expected UnsupportedBlock error, got {errors:?}"
        );
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
