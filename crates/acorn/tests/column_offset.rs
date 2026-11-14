//! Where acorn-typescript's errors placed at a column number point.
//!
//! It raises a few errors at `loc.column`, which acorn counts from
//! `lineStart`. Reading from a position, acorn starts `lineStart` after
//! the last `\n` before it (its constructor's `lastIndexOf("\n")`, or
//! the compiler's `startLocation`, which agrees), and moves it past
//! every line break it reads from there: `\r`, U+2028 and U+2029 too.
//! Each case is the error position acorn 8.19.0 with acorn-typescript
//! gives `parseExpressionAt(source, start)`, in UTF-16 units.

use svn_acorn::{Language, column_as_offset_from, parse_expression_at};

fn byte_of_unit(source: &str, unit: usize) -> u32 {
    let mut units = 0;
    for (i, c) in source.char_indices() {
        if units >= unit {
            return i as u32;
        }
        units += c.len_utf16();
    }
    source.len() as u32
}

#[test]
fn only_a_newline_before_the_start_begins_a_line() {
    for (source, start, want_unit) in [
        ("ab\rclass { static public x }", 3, 18),
        ("ab\nclass { static public x }", 3, 15),
        ("ab\u{2028}class { static public x }", 3, 18),
        ("a\rb\rclass { static public x }", 4, 19),
        ("a\nb\rclass { static public x }", 4, 17),
        ("(\rclass { static public x })", 0, 15),
        ("ab\r(\u{2028}class { static public x })", 3, 15),
        ("class { static public x }", 0, 15),
    ] {
        let start = source.char_indices().nth(start).map_or(0, |(i, _)| i) as u32;
        let error = parse_expression_at(source, start, Language::TypeScript)
            .expect_err("a modifier in the wrong order");
        assert!(error.column_of, "{source:?}");
        assert_eq!(
            column_as_offset_from(source, start, error.pos),
            byte_of_unit(source, want_unit),
            "{source:?}"
        );
    }
}
