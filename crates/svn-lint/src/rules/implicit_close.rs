//! `element_implicitly_closed` — HTML auto-close of an element whose
//! closing tag is optional.
//!
//! The Svelte compiler warns from its parser (`phases/1-parse/state/
//! element.js`) at the moment it ends an element early: a following open
//! tag the element may not contain (`<li>a<li>`, per the HTML
//! `closing_tag_omitted` table), or an ancestor's closing tag
//! (`<div><span>a</div>`). Only a plain element can be closed this way,
//! and only while it is the innermost open node: a component, a
//! `svelte:*` element or a block in between keeps the outer element open.
//!
//! svn-parser makes the same decision while building the tree and
//! records it on the element (`Element::implicit_close`), so this rule
//! only reports what the parser recorded.

use svn_core::Range;
use svn_parser::{Element, Fragment, Node};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

pub fn report(fragment: &Fragment, ctx: &mut LintContext<'_>) {
    let mut closed: Vec<&Element> = Vec::new();
    collect(fragment, &mut closed);
    // The compiler warns as it meets each trigger, in source order. One
    // closing tag can end several elements; it pops the innermost first,
    // which is the one that starts last.
    closed.sort_by_key(|e| {
        let trigger = e.implicit_close.as_ref().map_or(0, |c| c.trigger_start);
        (trigger, std::cmp::Reverse(e.range.start))
    });
    for element in closed {
        let Some(close) = &element.implicit_close else {
            continue;
        };
        let message =
            messages::element_implicitly_closed(&close.trigger, &format!("</{}>", element.name));
        // The compiler's range ends at the element's first child, or at
        // the trigger when it has none.
        let end = element
            .children
            .nodes
            .first()
            .map_or(close.trigger_start, |n| n.range().start);
        ctx.emit(
            Code::element_implicitly_closed,
            message,
            Range::new(element.range.start, end),
        );
    }
}

fn collect<'a>(fragment: &'a Fragment, out: &mut Vec<&'a Element>) {
    for node in &fragment.nodes {
        match node {
            Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
            Node::Element(e) => {
                if e.implicit_close.as_ref().is_some_and(|c| c.warns) {
                    out.push(e);
                }
                collect(&e.children, out);
            }
            Node::Component(c) => collect(&c.children, out),
            Node::SvelteElement(s) => collect(&s.children, out),
            Node::IfBlock(b) => {
                collect(&b.consequent, out);
                for arm in &b.elseif_arms {
                    collect(&arm.body, out);
                }
                if let Some(alt) = &b.alternate {
                    collect(alt, out);
                }
            }
            Node::EachBlock(b) => {
                collect(&b.body, out);
                if let Some(alt) = &b.alternate {
                    collect(alt, out);
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(pending) = &b.pending {
                    collect(pending, out);
                }
                if let Some(then) = &b.then_branch {
                    collect(&then.body, out);
                }
                if let Some(catch) = &b.catch_branch {
                    collect(&catch.body, out);
                }
            }
            Node::KeyBlock(b) => collect(&b.body, out),
            Node::SnippetBlock(b) => collect(&b.body, out),
        }
    }
}
