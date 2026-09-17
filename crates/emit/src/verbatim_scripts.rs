//! Which `<script>` blocks svelte2tsx recognises.
//!
//! svelte2tsx finds a component's script blocks with its own regular
//! expression (`utils/htmlxparser.ts` `scriptRegex`), not with the
//! Svelte parser: an opening `<script …>` tag, the shortest run of text
//! after it, and a literal `</script>`. Comments are skipped. A block
//! the Svelte parser accepts but the expression misses — `</script >`
//! with a space, say — is not a script to svelte2tsx: it processes no
//! script, and the block's text stays in the template output verbatim.

use svn_core::Range;

/// The source spans (opening `<` through the closing `>`) of the script
/// blocks svelte2tsx's expression matches, in source order.
pub(crate) fn recognised_script_spans(source: &str) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    let mut spans = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if source[i..].starts_with("<!--") {
            if let Some(end) = source[i + 4..].find("-->") {
                i = i + 4 + end + 3;
                continue;
            }
        } else if source[i..].starts_with("<script")
            && let Some(open_end) = open_tag_end(bytes, i + "<script".len())
            && let Some(close) = source[open_end..].find("</script>")
        {
            let end = open_end + close + "</script>".len();
            spans.push((i, end));
            i = end;
            continue;
        }
        i += 1;
    }
    spans
}

/// Match `(?:\s+NAME=(?:"…"|'…'|[^>\s]+)|\s+NAME)*\s*>` at `pos`, where
/// NAME is `[^=>'"/\s]+`; the offset after the `>`.
fn open_tag_end(bytes: &[u8], mut pos: usize) -> Option<usize> {
    let is_space = |b: u8| b.is_ascii_whitespace();
    loop {
        let save = pos;
        let mut j = pos;
        while j < bytes.len() && is_space(bytes[j]) {
            j += 1;
        }
        if j == pos {
            break;
        }
        let name_start = j;
        while j < bytes.len()
            && !matches!(bytes[j], b'=' | b'>' | b'\'' | b'"' | b'/')
            && !is_space(bytes[j])
        {
            j += 1;
        }
        if j == name_start {
            pos = save;
            break;
        }
        if bytes.get(j) == Some(&b'=') {
            let value_start = j + 1;
            let value_end = match bytes.get(value_start) {
                Some(&q @ (b'"' | b'\'')) => bytes[value_start + 1..]
                    .iter()
                    .position(|&b| b == q)
                    .map(|p| value_start + 1 + p + 1),
                _ => {
                    let mut k = value_start;
                    while k < bytes.len() && bytes[k] != b'>' && !is_space(bytes[k]) {
                        k += 1;
                    }
                    (k > value_start).then_some(k)
                }
            };
            match value_end {
                Some(end) => j = end,
                // `NAME` alone matched; the `=` then fails the tag.
                None => return None,
            }
        }
        pos = j;
    }
    while pos < bytes.len() && is_space(bytes[pos]) {
        pos += 1;
    }
    (bytes.get(pos) == Some(&b'>')).then_some(pos + 1)
}

/// Whether svelte2tsx recognises the script block spanning `open`
/// through `close`.
pub(crate) fn is_recognised(spans: &[(usize, usize)], open: Range, close: Range) -> bool {
    spans.contains(&(open.start as usize, close.end as usize))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_close_tag_is_required() {
        let src = "<script lang=\"ts\">let a = 1;</script>";
        assert_eq!(recognised_script_spans(src), vec![(0, src.len())]);
        assert!(recognised_script_spans("<script>let a;</script >").is_empty());
    }

    #[test]
    fn comments_and_attributes() {
        let src = "<!-- <script>x</script> --><script context=module a='>'>y</script>";
        assert_eq!(recognised_script_spans(src), vec![(27, src.len())]);
    }
}
