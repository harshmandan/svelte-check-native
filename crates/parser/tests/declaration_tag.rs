//! Where a `{let …}` / `{const …}` tag ends, or the error it throws.
//!
//! The compiler has acorn read one statement from the keyword and then
//! requires the `}` after white space. acorn ends the statement where
//! semicolon insertion does, which can be well before the `}`. Each case
//! is what svelte 5.57.3's `parse()` does with the template: the end of
//! the tag, or the error's code and offset.

#![allow(clippy::expect_used)]

use svn_parser::tag_read::{TagReadErrorKind, TagReader};

/// The end of the tag (`None` for an expression tag), or the error's
/// code and offset.
type Outcome = Result<Option<u32>, (&'static str, u32)>;

fn read(template: &str) -> Outcome {
    let reader = TagReader::new(template, false);
    let start = template.find(['l', 'c']).expect("a keyword") as u32;
    // The parser reads no further than the `}` a brace count finds.
    let limit = svn_parser::find_mustache_end(template, 1).unwrap_or(template.len() as u32);
    match reader.read_declaration_tag(start, limit) {
        Ok(read) => Ok(read.map(|read| read.end)),
        Err(error) => Err((
            match error.kind {
                TagReadErrorKind::ExpectedToken(_) => "expected_token",
                TagReadErrorKind::JsParse(_) => "js_parse_error",
                TagReadErrorKind::UnexpectedEof => "unexpected_eof",
                _ => "other",
            },
            error.at,
        )),
    }
}

#[test]
fn declaration_tags_end_where_acorn_ends_the_statement() {
    let cases: &[(&str, Outcome)] = &[
        ("{let x = 1\nfoo}", Err(("expected_token", 11))),
        ("{let x = 1;;}", Err(("expected_token", 11))),
        ("{let x = 1;}", Ok(Some(12))),
        ("{let x = 1}", Ok(Some(11))),
        ("{let x = 1 }", Ok(Some(12))),
        ("{let x = 1\n}", Ok(Some(12))),
        ("{const a = 1, b = 2}", Ok(Some(20))),
        ("{let x = 1 /* c */}", Err(("expected_token", 11))),
        ("{let x = 1 // c\n}", Err(("expected_token", 11))),
        ("{let x = 1", Err(("expected_token", 10))),
        ("{let x = 1\n\n  ", Err(("expected_token", 10))),
        ("{let x = (1}", Err(("js_parse_error", 11))),
        ("{let}", Err(("js_parse_error", 1))),
        ("{let x}", Ok(Some(7))),
        ("{let x = 1 foo}", Err(("js_parse_error", 11))),
        ("{let [a] = [1]}", Ok(Some(15))),
        ("{let x = (1", Err(("unexpected_eof", 11))),
    ];
    let mut failures = Vec::new();
    for (template, want) in cases {
        let got = read(template);
        if got != *want {
            failures.push(format!("{template:?}: got {got:?}, want {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
