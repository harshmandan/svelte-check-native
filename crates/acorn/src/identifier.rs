//! What acorn takes for a name, a keyword and a reserved word
//! (`identifier.js`), for ECMAScript 2025 module code.

use crate::identifier_tables::{
    ASTRAL_CONTINUE, ASTRAL_START, NON_ASCII_CONTINUE, NON_ASCII_START,
};

fn in_ranges(code: i32, ranges: &[(u16, u16)]) -> bool {
    let Ok(code) = u16::try_from(code) else {
        return false;
    };
    ranges
        .binary_search_by(|&(start, end)| {
            if code < start {
                std::cmp::Ordering::Greater
            } else if code > end {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Equal
            }
        })
        .is_ok()
}

fn in_astral_set(code: i32, set: &[u32]) -> bool {
    let code = i64::from(code);
    let mut pos: i64 = 0x10000;
    for pair in set.chunks_exact(2) {
        pos += i64::from(pair[0]);
        if pos > code {
            return false;
        }
        pos += i64::from(pair[1]);
        if pos >= code {
            return true;
        }
    }
    false
}

/// `isIdentifierStart(code, true)`. A `code` below zero is the end of
/// the input.
pub(crate) fn is_identifier_start(code: i32) -> bool {
    if code < 65 {
        return code == 36;
    }
    if code < 91 {
        return true;
    }
    if code < 97 {
        return code == 95;
    }
    if code < 123 {
        return true;
    }
    if code <= 0xffff {
        return code >= 0xaa && in_ranges(code, NON_ASCII_START);
    }
    in_astral_set(code, ASTRAL_START)
}

/// `isIdentifierChar(code, true)`.
pub(crate) fn is_identifier_char(code: i32) -> bool {
    if code < 48 {
        return code == 36;
    }
    if code < 58 {
        return true;
    }
    if code < 65 {
        return false;
    }
    if code < 91 {
        return true;
    }
    if code < 97 {
        return code == 95;
    }
    if code < 123 {
        return true;
    }
    if code <= 0xffff {
        return code >= 0xaa && in_ranges(code, NON_ASCII_CONTINUE);
    }
    in_astral_set(code, ASTRAL_START) || in_astral_set(code, ASTRAL_CONTINUE)
}

/// The words that are never names.
pub(crate) const KEYWORDS: &[&str] = &[
    "break",
    "case",
    "catch",
    "continue",
    "debugger",
    "default",
    "do",
    "else",
    "finally",
    "for",
    "function",
    "if",
    "return",
    "switch",
    "throw",
    "try",
    "var",
    "while",
    "with",
    "null",
    "true",
    "false",
    "instanceof",
    "typeof",
    "void",
    "delete",
    "new",
    "in",
    "this",
    "const",
    "class",
    "extends",
    "export",
    "import",
    "super",
];

pub(crate) fn is_keyword(word: &str) -> bool {
    KEYWORDS.contains(&word)
}

/// Reserved in a module, strict or not.
pub(crate) fn is_reserved(word: &str) -> bool {
    matches!(word, "enum" | "await")
}

/// Reserved in strict code.
pub(crate) fn is_reserved_strict(word: &str) -> bool {
    is_reserved(word)
        || matches!(
            word,
            "implements"
                | "interface"
                | "let"
                | "package"
                | "private"
                | "protected"
                | "public"
                | "static"
                | "yield"
        )
}

/// Reserved in strict code where a name is bound or assigned to.
pub(crate) fn is_reserved_strict_bind(word: &str) -> bool {
    is_reserved_strict(word) || matches!(word, "eval" | "arguments")
}
