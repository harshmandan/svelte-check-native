//! The JSDoc TypeScript attaches to a node, read the way
//! `ts.getJSDocTags` / `ts.getJSDocType` read it.

/// The tags of the JSDoc block TypeScript attaches to a node, as
/// `(tag name, tag text)` pairs — what `ts.getJSDocTags` reads.
///
/// A node's JSDoc comes from the `/** … */` comments between the end of
/// the token before it (`trivia_start`) and its own start. TypeScript
/// counts such a comment only after a line break outside any comment
/// (or at the very start of the file), except for the node kinds whose
/// same-line comments it reads too (`same_line`: variable declarations,
/// parameters, function expressions and arrows). Only the LAST block
/// contributes tags; earlier ones are ignored.
pub(crate) fn attached_jsdoc_tags<'s>(
    comments: &[oxc_ast::Comment],
    source: &'s str,
    trivia_start: usize,
    node_start: usize,
    same_line: bool,
) -> Vec<(&'s str, &'s str)> {
    let mut collecting = same_line || trivia_start == 0;
    let mut gap_start = trivia_start;
    let mut last: Option<&'s str> = None;
    for comment in comments {
        let (start, end) = (comment.span.start as usize, comment.span.end as usize);
        if start < trivia_start || end > node_start {
            continue;
        }
        collecting |= source
            .get(gap_start..start)
            .is_some_and(|gap| gap.contains(['\n', '\r']));
        gap_start = end;
        let Some(text) = source.get(start..end) else {
            continue;
        };
        if collecting && text.starts_with("/**") && !text.starts_with("/**/") {
            last = Some(text);
        }
    }
    last.map(jsdoc_block_tags).unwrap_or_default()
}

/// Split a `/** … */` block into its tags the way TypeScript's JSDoc
/// parser (`parseJSDocCommentWorker` / `parseTagComments`) starts them.
/// An `@` followed by a name opens a tag when it:
///
/// - starts a line, after optional whitespace and one `*` margin;
/// - follows whitespace — in a tag's text only outside a backtick span
///   (a line break closes one) and when a name follows directly;
/// - directly follows the name a `@see`, `@param`-like or `@author` tag
///   reads first (`@see Foo@type {T}` is two tags).
///
/// An inline `{@link …}` (`@linkcode`, `@linkplain`) runs to its `}`,
/// across lines, and holds no tags; other braces mean nothing here.
fn jsdoc_block_tags(block: &str) -> Vec<(&str, &str)> {
    let body = block
        .strip_prefix("/**")
        .and_then(|b| b.strip_suffix("*/"))
        .unwrap_or("");
    let bytes = body.as_bytes();
    let mut starts: Vec<(usize, usize)> = Vec::new();
    let mut line_start = true;
    let mut margin_seen = false;
    let mut after_whitespace = true;
    let mut in_tag = false;
    let mut in_backticks = false;
    let mut glued_at: Option<usize> = None;
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'\n' | b'\r' => {
                line_start = true;
                margin_seen = false;
                after_whitespace = true;
                in_backticks = false;
                i += 1;
                continue;
            }
            b' ' | b'\t' => {
                after_whitespace = true;
                i += 1;
                continue;
            }
            b'*' if line_start && !margin_seen => {
                margin_seen = true;
                i += 1;
                continue;
            }
            b'{' if inline_link_at(body, i) => {
                i = body[i..].find('}').map_or(bytes.len(), |e| i + e + 1);
            }
            b'`' if in_tag => {
                in_backticks = !in_backticks;
                i += 1;
            }
            b'@' => {
                let name_len = tag_name_len(&body[i + 1..]);
                let opens = line_start
                    || glued_at == Some(i)
                    || (after_whitespace && (!in_tag || (!in_backticks && name_len > 0)));
                if opens && name_len > 0 {
                    let name_end = i + 1 + name_len;
                    starts.push((i, name_end));
                    in_tag = true;
                    in_backticks = false;
                    glued_at = name_reference_end(&body[i + 1..name_end], body, name_end);
                    i = name_end;
                } else {
                    i += 1;
                }
            }
            _ => i += body[i..].chars().next().map_or(1, char::len_utf8),
        }
        line_start = false;
        after_whitespace = false;
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &(_, name_end))| {
            let text_end = starts.get(k + 1).map_or(body.len(), |&(next, _)| next);
            (&body[starts[k].0 + 1..name_end], &body[name_end..text_end])
        })
        .collect()
}

/// A tag name: the identifier after `@`.
fn tag_name_len(rest: &str) -> usize {
    rest.bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
        .count()
}

/// Whether `{` at `at` opens an inline `{@link …}`, `{@linkcode …}` or
/// `{@linkplain …}` tag.
fn inline_link_at(body: &str, at: usize) -> bool {
    let rest = &body[at + 1..];
    let rest = rest.trim_start_matches([' ', '\t']);
    ["@linkcode", "@linkplain", "@link"].iter().any(|tag| {
        rest.strip_prefix(tag).is_some_and(|after| {
            !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
        })
    })
}

/// Where the name a tag reads before its text ends, for the tags that
/// read one: `@see`'s name reference, the parameter name of `@param`,
/// `@arg`, `@argument`, `@property` and `@prop` (after an optional
/// `{type}`), and `@author`'s name (up to an `<email>`). TypeScript
/// starts the tag's text right there, so an `@` at that point opens the
/// next tag.
fn name_reference_end(tag: &str, body: &str, from: usize) -> Option<usize> {
    let bytes = body.as_bytes();
    let skip_blank = |mut i: usize| {
        while i < bytes.len() && matches!(bytes[i], b' ' | b'\t') {
            i += 1;
        }
        i
    };
    let entity = |mut i: usize| {
        while i < bytes.len()
            && (bytes[i].is_ascii_alphanumeric() || matches!(bytes[i], b'_' | b'$' | b'.' | b'#'))
        {
            i += 1;
        }
        i
    };
    let balanced = |i: usize, open: u8, close: u8| -> Option<usize> {
        let mut depth = 0usize;
        for (k, &b) in bytes.iter().enumerate().skip(i) {
            if b == open {
                depth += 1;
            } else if b == close {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return Some(k + 1);
                }
            }
        }
        None
    };
    match tag {
        "see" => {
            let start = skip_blank(from);
            let end = entity(start);
            (end > start).then_some(end)
        }
        "param" | "arg" | "argument" | "property" | "prop" => {
            let mut i = skip_blank(from);
            if bytes.get(i) == Some(&b'{') {
                i = skip_blank(balanced(i, b'{', b'}')?);
            }
            let end = if bytes.get(i) == Some(&b'[') {
                balanced(i, b'[', b']')?
            } else {
                entity(i)
            };
            (end > i).then_some(end)
        }
        "author" => {
            let start = skip_blank(from);
            let mut i = start;
            while i < bytes.len() && !matches!(bytes[i], b'<' | b'@' | b'\n' | b'\r') {
                i += 1;
            }
            if bytes.get(i) == Some(&b'<') {
                return body[i..].find('>').map(|e| i + e + 1);
            }
            (i > start).then_some(i)
        }
        _ => None,
    }
}

pub(crate) fn has_type_tag(tags: &[(&str, &str)]) -> bool {
    tags.iter().any(|(name, _)| *name == "type")
}

pub(crate) fn has_satisfies_tag(tags: &[(&str, &str)]) -> bool {
    tags.iter().any(|(name, _)| *name == "satisfies")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(body: &str) -> Vec<String> {
        let block = format!("/**{body}*/");
        jsdoc_block_tags(&block)
            .into_iter()
            .map(|(name, _)| name.to_string())
            .collect()
    }

    /// Each expectation is what TypeScript 6.0.3's `ts.getJSDocTags`
    /// returns for the same comment.
    #[test]
    fn tags_start_where_typescript_starts_them() {
        let cases: &[(&str, &[&str])] = &[
            (" @type {string} ", &["type"]),
            (" x @type {string} ", &["type"]),
            (" x@type {string} ", &[]),
            (" {@link x @type} ", &[]),
            (" {@link x} @type {string} ", &["type"]),
            (" {@linkcode x @type} ", &[]),
            (" {@linkplain x @type} ", &[]),
            (" {x @type} ", &["type"]),
            (" `x @type` ", &["type"]),
            (" `@type {string}` ", &[]),
            (" @see `x @type {string}` ", &["see"]),
            (" @see x@type {string} ", &["see", "type"]),
            (" @see x @type {string} ", &["see", "type"]),
            (" @see {@link x @type} ", &["see"]),
            (" @see {@link x} @type {string} ", &["see", "type"]),
            (" @param p x@type {string} ", &["param"]),
            (" @param p - x @type {string} ", &["param", "type"]),
            (" @param {string} p@type {string} ", &["param", "type"]),
            (" @deprecated x@type {string} ", &["deprecated"]),
            (" @deprecated @type {string} ", &["deprecated", "type"]),
            (" @returns x@type {string} ", &["returns"]),
            (" @typedef {object} T@type {string} ", &["typedef"]),
            (" a*@type {string} ", &[]),
            ("\n * a\n * @type {string}\n ", &["type"]),
            ("\n * `a\n * @type {string}\n * ` ", &["type"]),
            (" @example x @type {string} ", &["example", "type"]),
            (" @see {@link x @type} y @type {string} ", &["see", "type"]),
            (" @see x\n *  @type {string} ", &["see", "type"]),
            (" email me@type.com ", &[]),
            (" @author me@type {string} ", &["author", "type"]),
            (" @author Name <me@type> ", &["author"]),
            (" {@link x\n * @type {string} }", &[]),
        ];
        for (body, expected) in cases {
            assert_eq!(names(body), *expected, "{body:?}");
        }
    }
}
