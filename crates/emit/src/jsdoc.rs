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

/// Split a `/** … */` block into its tags. A tag starts at an `@` that
/// opens a line (after the optional `*` margin) or follows whitespace,
/// and runs until the next tag.
fn jsdoc_block_tags(block: &str) -> Vec<(&str, &str)> {
    let body = block
        .strip_prefix("/**")
        .and_then(|b| b.strip_suffix("*/"))
        .unwrap_or("");
    let bytes = body.as_bytes();
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'@' {
            continue;
        }
        let opens = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'*');
        let name_len = body[i + 1..]
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        if opens && name_len > 0 {
            starts.push((i, i + 1 + name_len));
        }
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

pub(crate) fn has_type_tag(tags: &[(&str, &str)]) -> bool {
    tags.iter().any(|(name, _)| *name == "type")
}

pub(crate) fn has_satisfies_tag(tags: &[(&str, &str)]) -> bool {
    tags.iter().any(|(name, _)| *name == "satisfies")
}
