//! The compiler's `decode_character_references`
//! (`phases/1-parse/utils/html.js`).
//!
//! The compiler decodes HTML character references in template text and
//! in static attribute values as it parses, so everything it checks
//! afterwards (a `role`, an `aria-*` value) sees `&#9;` as a tab and
//! `&nbsp;` as a no-break space.

use std::borrow::Cow;

use crate::html_entities::ENTITIES;

/// What code points 128–159 stand for: browsers read them as
/// Windows-1252.
const WINDOWS_1252: [u32; 32] = [
    8364, 129, 8218, 402, 8222, 8230, 8224, 8225, 710, 8240, 352, 8249, 338, 141, 381, 143, 144,
    8216, 8217, 8220, 8221, 8226, 8211, 8212, 732, 8482, 353, 8250, 339, 157, 382, 376,
];

/// `text` with its character references decoded. In an attribute value
/// a named reference without its `;` is left alone when a letter, a
/// digit, `_` or `=` follows it, and a line feed stays a line feed.
pub(crate) fn decode_character_references(text: &str, is_attribute_value: bool) -> Cow<'_, str> {
    if !text.contains('&') {
        return Cow::Borrowed(text);
    }
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        match reference(after, is_attribute_value) {
            Some((len, code)) => {
                let code = validate_code(code, is_attribute_value);
                out.push(char::from_u32(code).unwrap_or('\0'));
                rest = &after[len..];
            }
            None => {
                out.push('&');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    Cow::Owned(out)
}

/// The reference at the start of `text` (what follows an `&`): its
/// length and its code point. `None` when there is none, or when it
/// stands for code point 0, which the compiler leaves as written.
fn reference(text: &str, is_attribute_value: bool) -> Option<(usize, u32)> {
    if let Some(number) = text.strip_prefix('#') {
        let (digits, radix) = match number.strip_prefix(['x', 'X']) {
            Some(hex) => (hex, 16),
            None => (number, 10),
        };
        let count = digits
            .bytes()
            .take_while(|b| char::from(*b).is_digit(radix))
            .count();
        if count == 0 {
            return None;
        }
        let mut len = 1 + usize::from(radix == 16) + count;
        if text[len..].starts_with(';') {
            len += 1;
        }
        // A number too large to hold is no valid code point either way.
        let code = u32::from_str_radix(&digits[..count], radix).unwrap_or(u32::MAX);
        return (code != 0).then_some((len, code));
    }
    ENTITIES.iter().find_map(|&(name, code)| {
        let after = text.strip_prefix(name)?;
        if is_attribute_value && !name.ends_with(';') {
            let next = after.chars().next();
            if next.is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '=') {
                return None;
            }
        }
        Some((name.len(), code))
    })
}

/// The code point the compiler writes for `code`: 0 for one HTML
/// forbids.
fn validate_code(code: u32, is_attribute_value: bool) -> u32 {
    // Outside an attribute value a line feed collapses with the
    // whitespace around it anyway.
    if code == 10 && !is_attribute_value {
        return 32;
    }
    match code {
        0..=127 => code,
        128..=159 => WINDOWS_1252[(code - 128) as usize],
        160..=55295 => code,
        // UTF-16 surrogate halves.
        55296..=57343 => 0,
        57344..=65535 => code,
        // Supplementary multilingual and ideographic planes.
        65536..=196_607 => code,
        // Supplementary special-purpose plane.
        917_504..=917_631 | 917_760..=917_999 => code,
        _ => 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_what_the_compiler_decodes() {
        let attr = |s| decode_character_references(s, true).into_owned();
        assert_eq!(
            attr("&#9;button&#10;&#13;&#12;link&#9;"),
            "\tbutton\n\r\x0Clink\t"
        );
        assert_eq!(attr("button&nbsp;link"), "button\u{a0}link");
        assert_eq!(attr("lin&#x212A;"), "lin\u{212a}");
        assert_eq!(attr("a&amp;b"), "a&b");
        // Without its `;`, a named reference is kept before a word
        // character or `=`.
        assert_eq!(attr("?a=1&ampb=2"), "?a=1&ampb=2");
        assert_eq!(attr("?a=1&amp=2"), "?a=1&amp=2");
        assert_eq!(attr("a&amp b"), "a& b");
        assert_eq!(attr("&#128;"), "\u{20ac}");
        assert_eq!(attr("&#0;"), "&#0;");
        assert_eq!(attr("&bogus;"), "&bogus;");
        assert_eq!(
            decode_character_references("a&#10;b", false).into_owned(),
            "a b"
        );
    }
}
