//! `element_implicitly_closed` — HTML5 auto-close detection.
//!
//! Upstream emits this warning in the parser (`phases/1-parse/state/
//! element.js:110, 215`) whenever:
//!   1. a new opening tag is encountered AND the innermost open
//!      regular-element parent's closing tag is optional by HTML5
//!      rules per the vendored `closing_tag_omitted(parent, next)`
//!      table — fires on `parent` with `<next>` and `</parent>`, OR
//!   2. a closing tag `</NAME>` doesn't match the innermost open
//!      element; each popped non-matching ancestor fires with
//!      `</NAME>` and `</ancestor>`.
//!
//! Our svn-parser errors-out on mismatched closing tags instead of
//! reproducing upstream's tolerant parse + auto-close pops, so we
//! can't read this out of the AST. This module walks the source
//! directly with a minimal tag scanner: attribute-quote aware,
//! comment-skipping, script/style-raw-content aware, and
//! mustache-scope-boundary aware (control blocks reset the tag
//! stack so e.g. an `{#if}` doesn't bridge an auto-close across its
//! halves).

use smol_str::SmolStr;
use svn_core::Range;

use crate::codes::Code;
use crate::context::LintContext;
use crate::html5::closing_tag_omitted;
use crate::messages;
use crate::rules::util::is_void_element;

struct OpenTag {
    name: SmolStr,
    open_start: u32,
    open_end: u32,
}

/// Frame in the open-tag stack. `Block` boundaries represent
/// `{#if}` / `{#each}` / `{#await}` / `{#key}` / `{#snippet}` — an
/// outer element can't be auto-closed by an inner element across
/// one, matching upstream's parser stack layering (IfBlock etc.
/// aren't RegularElement, so `parent.type === 'RegularElement'`
/// gates the warning).
enum Frame {
    Open(OpenTag),
    Block,
}

pub fn scan(source: &str, ctx: &mut LintContext<'_>) {
    let bytes = source.as_bytes();
    let mut stack: Vec<Frame> = Vec::new();
    let mut i: usize = 0;
    let len = bytes.len();

    while i < len {
        let b = bytes[i];
        if b == b'<' {
            // Comment.
            if bytes.get(i + 1..i + 4) == Some(b"!--") {
                if let Some(end) = find_bytes(bytes, i + 4, b"-->") {
                    i = end + 3;
                } else {
                    i = len;
                }
                continue;
            }
            // CDATA.
            if bytes.get(i + 1..i + 9) == Some(b"![CDATA[") {
                if let Some(end) = find_bytes(bytes, i + 9, b"]]>") {
                    i = end + 3;
                } else {
                    i = len;
                }
                continue;
            }
            // Closing tag.
            if bytes.get(i + 1) == Some(&b'/') {
                let close_start = i + 2;
                let (name_end, name) = read_tag_name(bytes, close_start);
                let mut j = name_end;
                while j < len && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                // Find `>`.
                let gt = bytes.get(j).copied();
                if gt != Some(b'>') {
                    i = name_end;
                    continue;
                }
                i = j + 1;
                handle_close(&mut stack, &name, i as u32 - 1, ctx);
                continue;
            }
            // Opening tag (or malformed `<`).
            let name_start = i + 1;
            let (name_end, name) = read_tag_name(bytes, name_start);
            if name.is_empty() {
                i += 1;
                continue;
            }
            // Scan past the attribute list to `>` or `/>`.
            let Some((open_end, self_closing)) = scan_to_tag_close(bytes, name_end) else {
                i = len;
                continue;
            };
            let open_start = i as u32;
            // svelte:* tags aren't RegularElements in upstream's
            // model; they participate in their own layering. For the
            // auto-close table, only real HTML tag names matter.
            let is_regular = !name.starts_with("svelte:")
                && !name.contains('-')
                && is_lowercase_start(&name);
            let tag_name = name.clone();

            if is_regular {
                handle_open(&mut stack, &tag_name, open_start, open_end as u32, ctx);
            }

            i = open_end;

            if self_closing {
                // Don't push.
            } else if is_regular && is_void_element(&tag_name) {
                // Void element — don't push.
            } else if is_regular
                && (tag_name == "script" || tag_name == "style" || tag_name == "textarea")
            {
                // Raw-content element — skip to closing tag.
                let close_marker = format!("</{tag_name}");
                if let Some(close_pos) = find_ci_bytes(bytes, i, close_marker.as_bytes()) {
                    // Scan to the `>` of the close tag.
                    let mut k = close_pos + close_marker.len();
                    while k < len && bytes[k].is_ascii_whitespace() {
                        k += 1;
                    }
                    if bytes.get(k) == Some(&b'>') {
                        i = k + 1;
                    } else {
                        i = close_pos;
                    }
                } else {
                    i = len;
                }
            } else if is_regular {
                stack.push(Frame::Open(OpenTag {
                    name: tag_name,
                    open_start,
                    open_end: open_end as u32,
                }));
            }
            continue;
        }
        if b == b'{' {
            // Mustache — skip balanced, respecting string/template
            // literals so braces inside strings don't confuse the
            // counter. Also track control-block boundaries.
            let tag = scan_mustache(bytes, i);
            match tag.kind {
                MustacheKind::BlockOpen => {
                    stack.push(Frame::Block);
                }
                MustacheKind::BlockClose => {
                    // Pop everything up to and including the matching
                    // Frame::Block marker. Any Open frames above that
                    // mark as implicitly closed by the block boundary
                    // itself — but upstream doesn't fire here, so we
                    // just pop silently.
                    while let Some(f) = stack.pop() {
                        if matches!(f, Frame::Block) {
                            break;
                        }
                    }
                }
                MustacheKind::BlockMid => {
                    // `{:else}` / `{:catch}` / `{:then}` — reset
                    // element stack *within* the current block
                    // frame without popping the block itself.
                    let mut popped = Vec::new();
                    while let Some(f) = stack.pop() {
                        if matches!(f, Frame::Block) {
                            popped.push(f);
                            break;
                        }
                    }
                    // Put the Block back.
                    if popped.pop().is_some() {
                        stack.push(Frame::Block);
                    }
                }
                MustacheKind::Other => {}
            }
            i = tag.end;
            continue;
        }
        i += 1;
    }
    let _ = ctx;
}

struct MustacheTag {
    end: usize,
    kind: MustacheKind,
}

enum MustacheKind {
    BlockOpen,
    BlockMid,
    BlockClose,
    Other,
}

fn scan_mustache(bytes: &[u8], start: usize) -> MustacheTag {
    // Detect block open `{#` / mid `{:` / close `{/`.
    let kind = match bytes.get(start + 1).copied() {
        Some(b'#') => MustacheKind::BlockOpen,
        Some(b':') => MustacheKind::BlockMid,
        Some(b'/') => MustacheKind::BlockClose,
        _ => MustacheKind::Other,
    };
    let end = find_mustache_end(bytes, start);
    MustacheTag { end, kind }
}

fn find_mustache_end(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    let mut depth = 1usize;
    while i < len {
        let b = bytes[i];
        match b {
            b'\\' if i + 1 < len => {
                i += 2;
                continue;
            }
            b'"' | b'\'' => {
                i = skip_js_string(bytes, i, b);
                continue;
            }
            b'`' => {
                i = skip_js_template(bytes, i);
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                // Line comment.
                i += 2;
                while i < len && bytes[i] != b'\n' {
                    i += 1;
                }
                continue;
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i + 1 < len && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i = (i + 2).min(len);
                continue;
            }
            b'{' => {
                depth += 1;
            }
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return i + 1;
                }
            }
            _ => {}
        }
        i += 1;
    }
    len
}

fn skip_js_string(bytes: &[u8], start: usize, quote: u8) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    while i < len {
        match bytes[i] {
            b'\\' if i + 1 < len => i += 2,
            c if c == quote => return i + 1,
            _ => i += 1,
        }
    }
    len
}

fn skip_js_template(bytes: &[u8], start: usize) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    while i < len {
        match bytes[i] {
            b'\\' if i + 1 < len => i += 2,
            b'`' => return i + 1,
            b'$' if bytes.get(i + 1) == Some(&b'{') => {
                // Template expression — balanced braces.
                let mut depth = 1usize;
                i += 2;
                while i < len && depth > 0 {
                    match bytes[i] {
                        b'\\' if i + 1 < len => {
                            i += 2;
                            continue;
                        }
                        b'"' | b'\'' => {
                            i = skip_js_string(bytes, i, bytes[i]);
                            continue;
                        }
                        b'`' => {
                            i = skip_js_template(bytes, i);
                            continue;
                        }
                        b'{' => depth += 1,
                        b'}' => depth -= 1,
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ => i += 1,
        }
    }
    len
}

fn read_tag_name(bytes: &[u8], start: usize) -> (usize, SmolStr) {
    let len = bytes.len();
    let mut j = start;
    if j >= len || !is_tag_name_start(bytes[j]) {
        return (j, SmolStr::default());
    }
    j += 1;
    while j < len && is_tag_name_cont(bytes[j]) {
        j += 1;
    }
    let name = std::str::from_utf8(&bytes[start..j]).unwrap_or("");
    (j, SmolStr::from(name))
}

fn is_tag_name_start(b: u8) -> bool {
    b.is_ascii_alphabetic()
}
fn is_tag_name_cont(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b':' | b'-' | b'_' | b'.')
}
fn is_lowercase_start(name: &str) -> bool {
    name.chars().next().is_some_and(|c| c.is_ascii_lowercase())
}

/// Scan from the position right after the tag name to the `>` that
/// ends the opening tag. Returns `(end_of_open_tag, self_closing)`.
/// `end_of_open_tag` is one past the `>`.
fn scan_to_tag_close(bytes: &[u8], start: usize) -> Option<(usize, bool)> {
    let len = bytes.len();
    let mut i = start;
    while i < len {
        match bytes[i] {
            b'>' => return Some((i + 1, false)),
            b'/' if bytes.get(i + 1) == Some(&b'>') => return Some((i + 2, true)),
            b'"' => i = skip_attr_quote(bytes, i, b'"'),
            b'\'' => i = skip_attr_quote(bytes, i, b'\''),
            b'{' => i = find_mustache_end(bytes, i),
            _ => i += 1,
        }
    }
    None
}

fn skip_attr_quote(bytes: &[u8], start: usize, quote: u8) -> usize {
    let len = bytes.len();
    let mut i = start + 1;
    while i < len {
        let b = bytes[i];
        if b == b'{' {
            i = find_mustache_end(bytes, i);
            continue;
        }
        if b == quote {
            return i + 1;
        }
        i += 1;
    }
    len
}

fn find_bytes(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= bytes.len() {
        return None;
    }
    bytes[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

fn find_ci_bytes(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    if needle.is_empty() || from >= bytes.len() {
        return None;
    }
    let needle_lower: Vec<u8> = needle.iter().map(|b| b.to_ascii_lowercase()).collect();
    let hay = &bytes[from..];
    for i in 0..=hay.len().saturating_sub(needle.len()) {
        if hay[i..i + needle.len()]
            .iter()
            .map(|b| b.to_ascii_lowercase())
            .eq(needle_lower.iter().copied())
        {
            return Some(i + from);
        }
    }
    None
}

/// On an opening tag: if the innermost open regular-element parent's
/// `closing_tag_omitted(parent, this)` fires, emit the warning and
/// pop that parent.
fn handle_open(
    stack: &mut Vec<Frame>,
    name: &str,
    open_start: u32,
    open_end: u32,
    ctx: &mut LintContext<'_>,
) {
    if let Some(Frame::Open(parent)) = stack.last() {
        if closing_tag_omitted(parent.name.as_str(), Some(name)) {
            let full = messages::element_implicitly_closed(
                &format!("<{name}>"),
                &format!("</{}>", parent.name),
            );
            let range = Range::new(parent.open_start, parent.open_end);
            ctx.emit(Code::element_implicitly_closed, full, range);
            stack.pop();
        }
    }
    let _ = (open_start, open_end);
}

/// On a closing `</NAME>`: walk up the open-stack; each popped
/// non-matching regular element fires the warning keyed on
/// `</NAME>`, until we find the matching element (or hit a Block
/// boundary / empty stack).
fn handle_close(stack: &mut Vec<Frame>, name: &str, _close_end: u32, ctx: &mut LintContext<'_>) {
    while let Some(frame) = stack.pop() {
        match frame {
            Frame::Block => {
                stack.push(Frame::Block);
                return;
            }
            Frame::Open(parent) => {
                if parent.name.as_str() == name {
                    return;
                }
                let full = messages::element_implicitly_closed(
                    &format!("</{name}>"),
                    &format!("</{}>", parent.name),
                );
                let range = Range::new(parent.open_start, parent.open_end);
                ctx.emit(Code::element_implicitly_closed, full, range);
            }
        }
    }
}
