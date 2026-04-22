//! Rules that fire on control-flow blocks.

use svn_parser::ast::{AwaitBlock, EachBlock, Fragment, IfBlock, KeyBlock, Node};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

pub fn visit_if(b: &IfBlock, ctx: &mut LintContext<'_>) {
    visit_block_fragment_for_empty(&b.consequent, ctx);
    for arm in &b.elseif_arms {
        visit_block_fragment_for_empty(&arm.body, ctx);
    }
    if let Some(alt) = &b.alternate {
        visit_block_fragment_for_empty(alt, ctx);
    }
}

pub fn visit_each(b: &EachBlock, ctx: &mut LintContext<'_>) {
    visit_block_fragment_for_empty(&b.body, ctx);
    if let Some(alt) = &b.alternate {
        visit_block_fragment_for_empty(alt, ctx);
    }
}

pub fn visit_key(b: &KeyBlock, ctx: &mut LintContext<'_>) {
    visit_block_fragment_for_empty(&b.body, ctx);
}

pub fn visit_await(b: &AwaitBlock, ctx: &mut LintContext<'_>) {
    if let Some(pending) = &b.pending {
        visit_block_fragment_for_empty(pending, ctx);
    }
    if let Some(then) = &b.then_branch {
        visit_block_fragment_for_empty(&then.body, ctx);
    }
    if let Some(catch) = &b.catch_branch {
        visit_block_fragment_for_empty(&catch.body, ctx);
    }
}

/// `block_empty`: fires when a block body is exactly one
/// whitespace-only Text node (upstream heuristic — matches "user
/// hasn't started typing content yet").
fn visit_block_fragment_for_empty(frag: &Fragment, ctx: &mut LintContext<'_>) {
    if frag.nodes.len() == 1
        && let Node::Text(t) = &frag.nodes[0]
        && t.content.chars().all(char::is_whitespace)
    {
        let msg = messages::block_empty();
        ctx.emit(Code::block_empty, msg, t.range);
    }
}
