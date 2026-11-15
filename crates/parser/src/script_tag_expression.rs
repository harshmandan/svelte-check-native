//! Where the regular expressions svelte2tsx and the compiler's
//! `preprocess` use to find `<script>` blocks match.
//!
//! Both find a component's script blocks with a regular expression
//! (svelte2tsx `utils/htmlxparser.ts` `scriptRegex`, the compiler's
//! `preprocess/index.js` `regex_script_tags`), not with the Svelte
//! parser: an opening `<script …>` tag, the shortest run of text after
//! it, and a literal `</script>`. Comments are skipped. A block the
//! Svelte parser closes some other way — `</script >` with a space,
//! say — is either missed, or runs on to the next literal `</script>`.
//!
//! They differ on `<style>`. svelte2tsx searches for script and style
//! blocks in one pass and takes whichever opens first, so `<script>`
//! text inside a style block (a CSS comment naming it, say) is part of
//! that style. The compiler's `preprocess` looks for scripts alone, so
//! the same text opens a script there.

/// The script blocks the compiler's `preprocess` expression matches:
/// source spans, opening `<` through the closing `>`, in source order.
pub fn script_tag_expression_spans(source: &str) -> Vec<(usize, usize)> {
    verbatim_spans(source, false)
}

/// The script blocks svelte2tsx recognises (`findVerbatimElements`):
/// script and style blocks are matched in one pass, so `<script>` text
/// inside a style block opens nothing.
pub fn svelte2tsx_script_spans(source: &str) -> Vec<(usize, usize)> {
    verbatim_spans(source, true)
}

fn verbatim_spans(source: &str, styles_shadow_scripts: bool) -> Vec<(usize, usize)> {
    let bytes = source.as_bytes();
    // The compiler's `preprocess` also takes a self-closing `<script />`;
    // svelte2tsx's expressions don't.
    let self_closing = !styles_shadow_scripts;
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
        } else if let Some(end) = block_end(source, i, "script", self_closing) {
            spans.push((i, end));
            i = end;
            continue;
        } else if styles_shadow_scripts && let Some(end) = block_end(source, i, "style", false) {
            i = end;
            continue;
        }
        i += 1;
    }
    spans
}

/// The end of a `<TAG …>…</TAG>` block (or, when `self_closing`, a
/// `<TAG … />`) opening at `at`: just past the first literal close tag
/// after the opening tag.
fn block_end(source: &str, at: usize, tag: &str, self_closing: bool) -> Option<usize> {
    let after_name = at + 1 + tag.len();
    if !source[at..].strip_prefix('<')?.starts_with(tag) {
        return None;
    }
    let close = format!("</{tag}>");
    // No close tag after this point: an opening `>` there ends nothing.
    let last_close = source.rfind(&close);
    let (_, end) = crate::match_attributes_then(source, after_name, |q| {
        let rest = &source[q..];
        if self_closing && rest.starts_with("/>") {
            return Some(q + 2);
        }
        if !rest.starts_with('>') || last_close.is_none_or(|last| last < q + 1) {
            return None;
        }
        let len = source[q + 1..].find(&close)?;
        Some(q + 1 + len + close.len())
    })?;
    Some(end)
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
    fn script_text_in_a_style_block_opens_no_script_for_svelte2tsx() {
        let src =
            "<style>\n/* see the <script> block */\n</style>\n<script lang=\"ts\">let a;</script>";
        let real = src.find("<script lang").unwrap();
        assert_eq!(svelte2tsx_script_spans(src), vec![(real, src.len())]);
        // The compiler's preprocess has no style pass: its match opens in
        // the comment and runs on to the real close tag.
        let in_comment = src.find("<script>").unwrap();
        assert_eq!(
            script_tag_expression_spans(src),
            vec![(in_comment, src.len())]
        );
    }

    #[test]
    fn style_text_in_a_script_block_is_script_content() {
        let src = "<script lang=\"ts\">\n// see the <style> block\n</script>\n<style>p{}</style>";
        let end = src.find("</script>").unwrap() + "</script>".len();
        assert_eq!(svelte2tsx_script_spans(src), vec![(0, end)]);
    }

    #[test]
    fn tags_matched_by_backtracking() {
        let src = "<script x=\"a lang=ts\"c>let a: number = 1;</script>";
        assert_eq!(svelte2tsx_script_spans(src), vec![(0, src.len())]);
        assert_eq!(script_tag_expression_spans(src), vec![(0, src.len())]);
        // The compiler's preprocess takes a self-closing tag; svelte2tsx
        // looks on for a `>`.
        let src = "<script a=b/><p>{x}</p>";
        assert_eq!(script_tag_expression_spans(src), vec![(0, 13)]);
        assert!(svelte2tsx_script_spans(src).is_empty());
    }

    #[test]
    fn comments_and_attributes() {
        let src = "<!-- <script>x</script> --><script context=module a='>'>y</script>";
        assert_eq!(script_tag_expression_spans(src), vec![(27, src.len())]);
    }
}
