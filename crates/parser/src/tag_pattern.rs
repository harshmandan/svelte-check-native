//! The attribute list of the regular expressions that find `<script>`
//! and `<style>` tags outside the Svelte parser: svelte2tsx's
//! `scriptRegex` / `styleRegex`, the compiler's `preprocess`
//! `regex_script_tags` / `regex_style_tags`, and svelte-check's
//! `isTsSvelte`. After the tag name they all read
//!
//! ```text
//! (?:\s+NAME=(?:"[^"]*"|'[^']*'|[^>\s]+)|\s+NAME)*\s*
//! ```
//!
//! with NAME `[^=>'"/\s]+` and `\s` JavaScript's (`isTsSvelte` writes
//! the item as `\s+NAME(?:=VALUE)?`, which tries the same things in the
//! same order), followed by an expression-specific tail (`>`, `/>`, a
//! close tag).
//!
//! The expressions run on a backtracking engine, so which match they
//! find — and whether they find one — can depend on backtracking: a
//! quoted value can be re-read as an unquoted one, and an unquoted
//! value can give back characters (`a=b/>` ends with `/>`). The matcher
//! here tries the alternatives in the engine's order and returns the
//! first full match, as the engine does. Whether the rest of the
//! expression matches from a given point in the list never depends on
//! how the list got there, so each point is explored once.

use std::collections::HashSet;

use crate::scanner::is_svelte_whitespace;

/// Match the attribute list at `from`, then `\s*`, then the tail:
/// `tail(at)` is called with the offset after the `\s*` and returns its
/// result when the rest of the expression matches there. Returns where
/// the attribute list ends (before the `\s*`) and the tail's result,
/// for the match a backtracking engine finds first.
pub fn match_attributes_then<T>(
    source: &str,
    from: usize,
    mut tail: impl FnMut(usize) -> Option<T>,
) -> Option<(usize, T)> {
    // Points in the list from which no match followed.
    let mut failed: HashSet<usize> = HashSet::new();
    // Depth-first over points in the list, each with the ends of one
    // more item still to try, most preferred last.
    let mut stack: Vec<(usize, Vec<usize>)> = vec![(from, item_ends(source, from))];
    while let Some((at, ends)) = stack.last_mut() {
        if let Some(next) = ends.pop() {
            if !failed.contains(&next) {
                let more = item_ends(source, next);
                stack.push((next, more));
            }
            continue;
        }
        // No further item from here: the list ends.
        let at = *at;
        if let Some(found) = tail(skip_white_space(source, at)) {
            return Some((at, found));
        }
        failed.insert(at);
        stack.pop();
    }
    None
}

/// The ends of one item of the list starting at `at`, least preferred
/// first: the engine tries `NAME="…"`, `NAME='…'`, `NAME=` with the
/// longest unquoted value and then ever shorter ones, and last `NAME`
/// alone.
fn item_ends(source: &str, at: usize) -> Vec<usize> {
    let name_start = skip_white_space(source, at);
    if name_start == at {
        return Vec::new();
    }
    let name_end = source[name_start..]
        .find(|c: char| matches!(c, '=' | '>' | '\'' | '"' | '/') || is_svelte_whitespace(c))
        .map_or(source.len(), |len| name_start + len);
    if name_end == name_start {
        return Vec::new();
    }
    let mut ends = vec![name_end];
    if let Some(value) = source[name_end..].strip_prefix('=') {
        let value_start = name_end + 1;
        let unquoted_len = value
            .find(|c: char| c == '>' || is_svelte_whitespace(c))
            .unwrap_or(value.len());
        ends.extend(
            value[..unquoted_len]
                .char_indices()
                .map(|(i, c)| value_start + i + c.len_utf8()),
        );
        for quote in ['\'', '"'] {
            if let Some(body) = value.strip_prefix(quote)
                && let Some(len) = body.find(quote)
            {
                ends.push(value_start + 1 + len + 1);
            }
        }
    }
    ends
}

fn skip_white_space(source: &str, at: usize) -> usize {
    source.len() - source[at..].trim_start_matches(is_svelte_whitespace).len()
}

#[cfg(test)]
mod tests {
    use super::match_attributes_then;

    fn open_tag(source: &str) -> Option<usize> {
        match_attributes_then(source, "<script".len(), |at| {
            source[at..].starts_with('>').then_some(at + 1)
        })
        .map(|(_, end)| end)
    }

    #[test]
    fn reads_names_and_values() {
        assert_eq!(open_tag("<script>"), Some(8));
        assert_eq!(open_tag("<script lang=\"ts\" a b='c' d=e >x"), Some(31));
        assert_eq!(open_tag("<script lang=\"ts\"\u{a0}>"), Some(20));
        assert_eq!(open_tag("<script a=>"), None);
    }

    #[test]
    fn a_quoted_value_is_read_again_unquoted() {
        // `"a lang=ts"c` cannot be one quoted value followed by `c`; read
        // unquoted, it is `x="a`, then `lang=ts"c`.
        let source = "<script x=\"a lang=ts\"c>";
        let (attrs_end, _) =
            match_attributes_then(source, 7, |at| source[at..].starts_with('>').then_some(()))
                .unwrap();
        assert_eq!(attrs_end, source.len() - 1);
    }

    #[test]
    fn an_unquoted_value_gives_back_a_slash_for_self_closing() {
        let source = "<script a=b/>";
        let found =
            match_attributes_then(source, 7, |at| source[at..].starts_with("/>").then_some(at));
        assert_eq!(found, Some((11, 11)));
    }

    #[test]
    fn failing_lists_take_linear_work() {
        // Every quoted value can also be read unquoted; without a record
        // of the points already explored this is 2^n.
        let source = format!("<style{} />", " a=\"1\"".repeat(64));
        let mut calls = 0;
        let found = match_attributes_then(&source, 6, |at| {
            calls += 1;
            source[at..].starts_with('>').then_some(())
        });
        assert!(found.is_none());
        // One try per point in the list: after a name, and after each
        // character of a value read unquoted.
        assert!(calls <= 5 * 64, "{calls}");
    }
}
