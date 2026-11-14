//! acorn, the JavaScript parser the Svelte compiler reads scripts and
//! template expressions with, carried over to Rust.
//!
//! The Svelte compiler reports a script's first syntax error as acorn
//! raises it: its message, and where. Which error comes first, how it
//! is worded and where it points all follow from how acorn happens to
//! be written, so the only way to say the same thing in every case is
//! to be the same parser. This crate is acorn's tokenizer and parser,
//! function for function, with the settings the compiler uses: module
//! code, ECMAScript 2025, parentheses kept. It builds only as much of
//! a syntax tree as acorn itself looks at again.
//!
//! Positions are byte offsets into the text given. Inside, the parser
//! indexes UTF-16 code units, as acorn does.

mod expression;
mod identifier;
mod identifier_tables;
mod lval;
mod regexp;
mod state;
mod statement;
mod token;
mod tokenizer;

use state::{ForInit, Parser, Stop};

/// The error acorn throws: its message without the `(line:column)`
/// suffix, and the byte offset it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub pos: u32,
}

/// A text as UTF-16 code units, with the byte offset of each.
struct Units {
    units: Vec<u16>,
    /// `byte_of[i]` is where unit `i` starts; one more entry gives the
    /// text's length.
    byte_of: Vec<u32>,
}

impl Units {
    fn new(text: &str) -> Units {
        let mut units = Vec::with_capacity(text.len());
        let mut byte_of = Vec::with_capacity(text.len() + 1);
        for (offset, c) in text.char_indices() {
            let mut buffer = [0u16; 2];
            for unit in c.encode_utf16(&mut buffer) {
                units.push(*unit);
                byte_of.push(offset as u32);
            }
        }
        byte_of.push(text.len() as u32);
        Units { units, byte_of }
    }

    fn byte(&self, unit: u32) -> u32 {
        let last = self.byte_of.len() - 1;
        self.byte_of[(unit as usize).min(last)]
    }

    /// The unit index of the byte offset `byte`, a character boundary.
    fn unit(&self, byte: u32) -> usize {
        self.byte_of.partition_point(|&b| b < byte)
    }

    fn error(&self, stop: Stop) -> SyntaxError {
        match stop {
            Stop::Error(raised) => SyntaxError {
                message: raised.message,
                pos: self.byte(raised.pos),
            },
            // Never leaves the tokenizer, which reads the template
            // again as an invalid one.
            Stop::InvalidTemplateEscape => SyntaxError {
                message: "Unexpected token".to_string(),
                pos: 0,
            },
        }
    }
}

/// How the compiler has acorn read a text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reading {
    /// A component's `<script>`: it may export a name the component
    /// declares elsewhere.
    Script,
    /// A module on its own, read from the start of its file: every
    /// export needs its declaration, and a first line starting `#!` is
    /// a comment.
    Module,
}

fn parser_for(text: &Units, pos: usize) -> Parser {
    Parser::new(text.units.clone(), pos)
}

/// `acorn.parse(source)`: the error acorn throws for the program, if
/// any.
///
/// A component's script never starts at offset zero of what the
/// compiler hands acorn, so a `#!` line is no comment there.
pub fn parse(source: &str, reading: Reading) -> Result<(), SyntaxError> {
    let text = Units::new(source);
    let mut parser = parser_for(&text, 0);
    parser.forget_undefined_exports = reading == Reading::Script;
    if reading == Reading::Module && source.starts_with("#!") {
        parser.skip_line_comment(2);
    }
    parser
        .next_token()
        .and_then(|()| parser.parse_top_level())
        .map_err(|stop| text.error(stop))
}

/// `acorn.parseExpressionAt(source, pos)`: where the expression that
/// starts at `pos` ends, or the error acorn throws reading it.
///
/// acorn reads the longest expression it can and one token past it, so
/// an error in that next token is an error here too.
pub fn parse_expression_at(source: &str, pos: u32) -> Result<u32, SyntaxError> {
    let text = Units::new(source);
    let mut parser = parser_for(&text, text.unit(pos));
    parser.preserve_parens = true;
    parser
        .next_token()
        .and_then(|()| parser.parse_expression(ForInit::No, None))
        .map(|node| text.byte(node.end))
        .map_err(|stop| text.error(stop))
}

/// The compiler's `parse_statement_at`: one statement read at `pos` as
/// a module's top-level statement. Returns where it ends.
pub fn parse_statement_at(source: &str, pos: u32) -> Result<u32, SyntaxError> {
    let text = Units::new(source);
    let mut parser = parser_for(&text, text.unit(pos));
    parser
        .next_token()
        .and_then(|()| parser.parse_statement(None, true))
        .map(|node| text.byte(node.end))
        .map_err(|stop| text.error(stop))
}

/// acorn's reason for rejecting the regular expression `pattern` with
/// `flags`, or `None` when it accepts it. The flags are taken as valid.
pub fn regexp_pattern_error(pattern: &str, flags: &str) -> Option<&'static str> {
    let units: Vec<u16> = pattern.encode_utf16().collect();
    regexp::pattern_error(&units, flags)
}
