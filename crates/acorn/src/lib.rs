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
//! Which acorn and which acorn-typescript it follows is pinned by
//! commit in `tests/corpus/versions.json`, next to the answers those
//! two gave for the corpus the tests hold the port to. acorn-typescript
//! is followed at its source, which may be ahead of its last release.
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
mod ts;
mod ts_expression;
mod ts_statement;
mod ts_types;

use state::{ForInit, Parser, Stop};

/// The error acorn throws: its message without the `(line:column)`
/// suffix, and the byte offset it points at.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyntaxError {
    pub message: String,
    pub pos: u32,
    /// acorn-typescript places a few errors at a column number that it
    /// takes for an offset. Where set, the error points at the offset
    /// that equals the column of `pos` in the text the compiler parsed.
    pub column_of: bool,
    /// The parser failed without an offset: either it raised with
    /// something that is none, or it broke on a node it did not have and
    /// `message` is the JavaScript engine's own. The compiler reports
    /// such an error with no position, and `pos` means nothing.
    pub no_position: bool,
}

/// A text as UTF-16 code units, with the byte offset of each.
struct Units {
    units: std::rc::Rc<[u16]>,
    /// `byte_of[i]` is where unit `i` starts; one more entry gives the
    /// text's length.
    byte_of: Vec<u32>,
}

impl Units {
    fn new(text: &str) -> Units {
        Units::window(text, 0)
    }

    /// `text`, which starts at byte `base` of the text offsets are
    /// counted in.
    fn window(text: &str, base: u32) -> Units {
        let mut units = Vec::with_capacity(text.len());
        let mut byte_of = Vec::with_capacity(text.len() + 1);
        for (offset, c) in text.char_indices() {
            let mut buffer = [0u16; 2];
            for unit in c.encode_utf16(&mut buffer) {
                units.push(*unit);
                byte_of.push(base + offset as u32);
            }
        }
        byte_of.push(base + text.len() as u32);
        Units {
            units: units.into(),
            byte_of,
        }
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
            Stop::Error(raised) | Stop::StackExhausted(raised) => SyntaxError {
                message: raised.message,
                pos: self.byte(raised.pos),
                column_of: raised.column_of,
                no_position: raised.no_position,
            },
            Stop::Crash(message) => SyntaxError {
                message: message.to_string(),
                pos: 0,
                column_of: false,
                no_position: true,
            },
            // Never leaves the tokenizer, which reads the template
            // again as an invalid one.
            Stop::InvalidTemplateEscape => SyntaxError {
                message: "Unexpected token".to_string(),
                pos: 0,
                column_of: false,
                no_position: false,
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

/// Which parser the compiler uses: acorn for a JavaScript component,
/// acorn with the acorn-typescript plugin for one with `lang="ts"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    JavaScript,
    TypeScript,
}

fn parser_for(text: &Units, pos: usize, language: Language) -> Parser {
    let mut parser = Parser::new(text.units.clone(), pos);
    parser.ts = language == Language::TypeScript;
    parser
}

/// `acorn.parse(source)`: the error acorn throws for the program, if
/// any.
///
/// A component's script never starts at offset zero of what the
/// compiler hands acorn, so a `#!` line is no comment there.
pub fn parse(source: &str, reading: Reading, language: Language) -> Result<(), SyntaxError> {
    let text = Units::new(source);
    let mut parser = parser_for(&text, 0, language);
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
pub fn parse_expression_at(source: &str, pos: u32, language: Language) -> Result<u32, SyntaxError> {
    Text::new(source).parse_expression_at(pos, language)
}

/// [`parse_expression_at`] on the text that ends at `end`: what
/// `parseExpressionAt(source.slice(0, end), pos)` answers.
///
/// acorn never looks behind where it starts, so only the text between
/// the two is prepared. Offsets stay those of `source`.
pub fn parse_expression_within(
    source: &str,
    pos: u32,
    end: u32,
    language: Language,
) -> Result<u32, SyntaxError> {
    let window = source.get(pos as usize..end as usize).unwrap_or("");
    Text {
        units: Units::window(window, pos),
    }
    .parse_expression_at(pos, language)
}

/// A text prepared for reading at any number of places. Preparing it
/// costs as much as the text is long; a read then costs what it reads.
pub struct Text {
    units: Units,
}

impl Text {
    pub fn new(source: &str) -> Text {
        Text {
            units: Units::new(source),
        }
    }

    /// [`parse_expression_at`] on this text.
    pub fn parse_expression_at(&self, pos: u32, language: Language) -> Result<u32, SyntaxError> {
        let text = &self.units;
        let mut parser = parser_for(text, text.unit(pos), language);
        parser.preserve_parens = true;
        parser
            .next_token()
            .and_then(|()| parser.parse_expression(ForInit::No, None))
            .map(|node| text.byte(node.end))
            .map_err(|stop| text.error(stop))
    }
}

/// `parseExpressionAt` on a text from its start, for a caller that has
/// to carry on where acorn stopped: where the expression ends, and
/// where the token after it starts (acorn has read that far, comments
/// included).
pub fn read_expression(source: &str, language: Language) -> Result<(u32, u32), SyntaxError> {
    let text = Units::new(source);
    let mut parser = parser_for(&text, 0, language);
    parser.preserve_parens = true;
    parser
        .next_token()
        .and_then(|()| parser.parse_expression(ForInit::No, None))
        .map(|node| (text.byte(node.end), text.byte(parser.start as u32)))
        .map_err(|stop| text.error(stop))
}

/// The compiler's `parse_statement_at`: one statement read at `pos` as
/// a module's top-level statement. Returns where it ends.
pub fn parse_statement_at(source: &str, pos: u32, language: Language) -> Result<u32, SyntaxError> {
    let text = Units::new(source);
    let mut parser = parser_for(&text, text.unit(pos), language);
    parser
        .next_token()
        .and_then(|()| parser.parse_statement(None, true))
        .map(|node| text.byte(node.end))
        .map_err(|stop| text.error(stop))
}

/// Where acorn's tokenizer finds the `}` that closes a tag whose
/// content starts `text`: the offset of the first `}` that pairs with
/// no `{` or `${` before it.
///
/// The tokenizer decides as it goes whether a `/` divides or starts a
/// regular expression, so a `}` inside a regular expression, a string
/// or a template's text closes nothing.
///
/// `Err` when no such `}` is found: `true` when the tokenizer read to
/// the end of `text`, where more text might still hold one, `false`
/// when it stopped at something it cannot read.
pub fn closing_brace(text: &str) -> Result<u32, bool> {
    let units = Units::new(text);
    let mut parser = parser_for(&units, 0, Language::TypeScript);
    let mut depth = 0u32;
    // The token before the current one: whether an expression could
    // start after it, and whether it is `await` or `yield`.
    let mut expression_ended = false;
    let mut awaits = false;
    let mut after_dot = false;
    let mut result = parser.next_token();
    loop {
        // Twice the parser overrules the tokenizer. Where it wants an
        // operand and is handed a `/`, it reads a regular expression:
        // the tokenizer takes `await` and `yield` for names, after
        // which it divides. And TypeScript's `!` after an expression
        // asserts instead of negating, so that a `/` after it divides.
        if result.is_ok() && parser.ty == token::Tok::Slash && awaits {
            result = parser.read_regexp();
        }
        if result.is_err() {
            return Err(parser.pos >= units.units.len());
        }
        if parser.ty == token::Tok::Prefix
            && parser.value == "!"
            && expression_ended
            && !awaits
            && !parser.has_preceding_line_break()
        {
            parser.expr_allowed = false;
        }
        match parser.ty {
            token::Tok::Eof => return Err(true),
            token::Tok::BraceL | token::Tok::DollarBraceL => depth += 1,
            token::Tok::BraceR if depth == 0 => return Ok(units.byte(parser.start as u32)),
            token::Tok::BraceR => depth -= 1,
            _ => {}
        }
        expression_ended = !parser.expr_allowed;
        // After a `.` they name a property.
        awaits = parser.ty == token::Tok::Name
            && !parser.contains_esc
            && !after_dot
            && matches!(parser.value.as_str(), "await" | "yield");
        after_dot = matches!(parser.ty, token::Tok::Dot | token::Tok::QuestionDot);
        result = parser.next();
    }
}

/// Where an error with `column_of` set points: acorn-typescript raised
/// it at the column number of `pos`, taken for an offset into `source`.
///
/// The column counts UTF-16 units from the last line break before `pos`
/// (`\n`, `\r`, U+2028 or U+2029, wherever it stands), and the offset
/// it is taken for counts them from the start of the text. That is
/// acorn's count when no `\r`, U+2028 or U+2029 stands before the
/// position the read started at; [`column_as_offset_from`] knows where
/// that was.
pub fn column_as_offset(source: &str, pos: u32) -> u32 {
    column_as_offset_from(source, 0, pos)
}

/// [`column_as_offset`] for a read that started at `start`. acorn's
/// `lineStart` begins after the last `\n` before `start` (its
/// constructor's `lastIndexOf("\n", startPos - 1)`: the line breaks it
/// never read are found by that alone), and moves past every line break
/// it reads from `start` on, `\r`, U+2028 and U+2029 included.
pub fn column_as_offset_from(source: &str, start: u32, pos: u32) -> u32 {
    let before = source.get(..pos as usize).unwrap_or(source);
    let start = (start as usize).min(before.len());
    let read = before.get(start..).unwrap_or("");
    let line_start = read
        .char_indices()
        .rev()
        .find(|(_, c)| matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}'))
        .map(|(i, c)| start + i + c.len_utf8())
        .or_else(|| before.get(..start)?.rfind('\n').map(|i| i + 1))
        .unwrap_or(0);
    let column = before[line_start..].encode_utf16().count();
    let mut units = 0;
    for (i, c) in source.char_indices() {
        if units >= column {
            return i as u32;
        }
        units += c.len_utf16();
    }
    source.len() as u32
}

/// acorn's reason for rejecting the regular expression `pattern` with
/// `flags`, or `None` when it accepts it. The flags are taken as valid.
pub fn regexp_pattern_error(pattern: &str, flags: &str) -> Option<&'static str> {
    let units: Vec<u16> = pattern.encode_utf16().collect();
    regexp::pattern_error(&units, flags)
}

/// What `regexp_pattern_error` answers for a pattern nested deeper than
/// acorn has stack for. acorn reports it as it stands, not as an
/// invalid regular expression.
pub const REGEXP_STACK_EXHAUSTED: &str = regexp::STACK_EXHAUSTED;
