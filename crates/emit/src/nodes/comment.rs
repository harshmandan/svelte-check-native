//! Comment-node handling.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Comment.ts`.
//!
//! Upstream's handler erases comment bytes and threads "leading" /
//! "trailing" comments onto adjacent element nodes so the overlay's
//! identifier-position blame accounts for source comments. The
//! transformation is editor-tooling-driven (hover positions stay
//! correct in the IDE).
//!
//! HTML comments need nothing: our overlay is built structurally rather
//! than by overwriting source bytes, so they are never emitted (see
//! `lib.rs::emit_template_node`'s `Node::Comment(_)` arm). JS comments
//! written inside a start tag do matter — a `// @ts-expect-error` there
//! annotates the attribute below it — so the helpers here write the
//! comments analyze threaded onto an attribute
//! ([`svn_analyze::CommentThread`]) around that attribute's code.
//!
//! `<!-- @component -->` is consumed at the svelte2tsx transform stage —
//! upstream's `nodes/ComponentDocumentation.ts` strips the tag and emits
//! the text as a leading JSDoc on the default-export component (IDE hover
//! only). It carries no type-check-surface effect, so we emit nothing.
//! `<!-- @hmr-keep -->` is a compiler/HMR-runtime concern, likewise
//! irrelevant to type checking.
//!
//! This file exists for parity navigation: a contributor familiar with
//! upstream's `Comment.ts` should land here.

use crate::emit_buffer::EmitBuffer;

/// Write the comments leading an attribute, each on its own line when
/// it started one in the source, then break the line so the attribute
/// code that follows is the line a TS comment directive annotates.
pub(crate) fn write_leading_comments(
    buf: &mut EmitBuffer,
    source: &str,
    thread: &svn_analyze::CommentThread,
) {
    if thread.leading.is_empty() {
        return;
    }
    for c in &thread.leading {
        if c.newline {
            buf.push('\n');
        }
        buf.append_with_source(c.range.slice(source), c.range);
    }
    buf.push('\n');
}

/// Write the comments trailing a tag's last attribute after its code,
/// then break the line so a `//` comment cannot swallow what follows.
pub(crate) fn write_trailing_comments(
    buf: &mut EmitBuffer,
    source: &str,
    thread: &svn_analyze::CommentThread,
) {
    if thread.trailing.is_empty() {
        return;
    }
    for c in &thread.trailing {
        buf.push(if c.newline { '\n' } else { ' ' });
        buf.append_with_source(c.range.slice(source), c.range);
    }
    buf.push('\n');
}
