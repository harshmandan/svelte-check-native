//! Where the compiler's `match_bracket()` reports a bracket that never
//! closes: at the end of the template, which the compiler trims at its
//! end before reading it. Each case is what svelte 5.57.3's `parse()`
//! throws: `unexpected_eof` at that offset.

#![allow(clippy::expect_used)]

use svn_parser::tag_read::{TagReadErrorKind, TagReader};

fn eof_at(error: svn_parser::tag_read::TagReadError) -> Option<u32> {
    (error.kind == TagReadErrorKind::UnexpectedEof).then_some(error.at)
}

#[test]
fn an_unclosed_each_pattern_ends_at_the_trimmed_template() {
    for (template, want) in [
        ("{#each items as {a\n", 18),
        ("{#each items as [a, {b\n  \n", 22),
    ] {
        let reader = TagReader::new(template, false);
        let got = reader
            .read_each_open(6, template.len() as u32)
            .err()
            .and_then(eof_at);
        assert_eq!(got, Some(want), "{template:?}");
    }
}

#[test]
fn unclosed_snippet_type_parameters_end_at_the_trimmed_template() {
    let template = "<script lang=\"ts\"></script>{#snippet s<T(a)}\n\n";
    let keyword_end = template.find("snippet").expect("a snippet") as u32 + 7;
    let reader = TagReader::new(template, true);
    let got = reader.read_snippet_open(keyword_end).err().and_then(eof_at);
    assert_eq!(got, Some(44), "{template:?}");
}
