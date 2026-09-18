//! Where the regular expression svelte2tsx and the compiler's
//! `preprocess` use to find `<script>` blocks matches.
//!
//! Both find a component's script blocks with a regular expression
//! (svelte2tsx `utils/htmlxparser.ts` `scriptRegex`, the compiler's
//! `preprocess/index.js` `regex_script_tags`), not with the Svelte
//! parser: an opening `<script …>` tag, the shortest run of text after
//! it, and a literal `</script>`. Comments are skipped. A block the
//! Svelte parser closes some other way — `</script >` with a space,
//! say — is either missed, or runs on to the next literal `</script>`.

/// The source spans (opening `<` through the closing `>`) of the script
/// blocks svelte2tsx's expression matches, in source order.
pub fn script_tag_expression_spans(source: &str) -> Vec<(usize, usize)> {
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

/// Where TypeScript's scanner ends a regular expression starting at the
/// `/` at `slash`: after its closing `/` and flags, or at the line
/// break (or end of text) when it is unterminated.
///
/// A close tag swallowed by a script body (`</script >`) is read as one:
/// the `/` after its `<` starts the expression.
pub fn typescript_regex_end(content: &str, slash: usize) -> usize {
    let bytes = content.as_bytes();
    let mut i = slash + 1;
    let mut in_class = false;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' | b'\r' => return i,
            b'\\' => i += 1,
            b'[' => in_class = true,
            b']' => in_class = false,
            b'/' if !in_class => {
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_') {
                    i += 1;
                }
                return i;
            }
            _ => {}
        }
        i += 1;
    }
    bytes.len().min(i)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn literal_close_tag_is_required() {
        let src = "<script lang=\"ts\">let a = 1;</script>";
        assert_eq!(script_tag_expression_spans(src), vec![(0, src.len())]);
        assert!(script_tag_expression_spans("<script>let a;</script >").is_empty());
    }

    #[test]
    fn regular_expression_ends() {
        assert_eq!(typescript_regex_end("</script >\nx", 1), 10);
        assert_eq!(typescript_regex_end("</script ></b>", 1), 13);
        assert_eq!(typescript_regex_end("/[/]x/g;", 0), 7);
    }

    #[test]
    fn comments_and_attributes() {
        let src = "<!-- <script>x</script> --><script context=module a='>'>y</script>";
        assert_eq!(script_tag_expression_spans(src), vec![(27, src.len())]);
    }
}
