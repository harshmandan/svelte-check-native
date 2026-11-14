//! acorn's tokenizer (`tokenize.js`) and the context updates that
//! steer it (`tokencontext.js`).

use crate::identifier::{is_identifier_char, is_identifier_start};
use crate::regexp;
use crate::state::{PResult, Parser, Stop, is_new_line};
use crate::token::{Ctx, Tok};

fn code_point_to_units(code: u32, out: &mut Vec<u16>) {
    if code <= 0xffff {
        out.push(code as u16);
    } else {
        let code = code - 0x10000;
        out.push(((code >> 10) + 0xd800) as u16);
        out.push(((code & 1023) + 0xdc00) as u16);
    }
}

fn code_point_to_string(code: i32) -> String {
    let mut units = Vec::new();
    code_point_to_units(code.max(0) as u32, &mut units);
    String::from_utf16_lossy(&units)
}

fn has_lone_surrogate(units: &[u16]) -> bool {
    char::decode_utf16(units.iter().copied()).any(|c| c.is_err())
}

impl Parser {
    // --- tokencontext.js ---

    pub(crate) fn cur_context(&self) -> Option<Ctx> {
        self.context.last().copied()
    }

    fn brace_is_block(&self, prev: Tok) -> bool {
        let parent = self.cur_context();
        if parent == Some(Ctx::FExpr) || parent == Some(Ctx::FStat) {
            return true;
        }
        if prev == Tok::Colon && (parent == Some(Ctx::BStat) || parent == Some(Ctx::BExpr)) {
            return parent == Some(Ctx::BStat);
        }
        if prev == Tok::KwReturn || (prev == Tok::Name && self.expr_allowed) {
            return self.line_break_between(self.last_tok_end, self.start);
        }
        if matches!(
            prev,
            Tok::KwElse | Tok::Semi | Tok::Eof | Tok::ParenR | Tok::Arrow
        ) {
            return true;
        }
        if prev == Tok::BraceL {
            return parent == Some(Ctx::BStat);
        }
        if matches!(prev, Tok::KwVar | Tok::KwConst | Tok::Name) {
            return false;
        }
        !self.expr_allowed
    }

    fn in_generator_context(&self) -> bool {
        self.context
            .iter()
            .skip(1)
            .rev()
            .find(|context| context.is_function())
            .is_some_and(|context| context.generator())
    }

    fn update_context(&mut self, prev: Tok) {
        let ty = self.ty;
        if ty.keyword().is_some() && prev == Tok::Dot {
            self.expr_allowed = false;
            return;
        }
        match ty {
            Tok::ParenR | Tok::BraceR => {
                if self.context.len() == 1 {
                    self.expr_allowed = true;
                    return;
                }
                let mut out = self.context.pop();
                if out == Some(Ctx::BStat) && self.cur_context().is_some_and(Ctx::is_function) {
                    out = self.context.pop();
                }
                self.expr_allowed = !out.is_some_and(Ctx::is_expr);
            }
            Tok::BraceL => {
                let context = if self.brace_is_block(prev) {
                    Ctx::BStat
                } else {
                    Ctx::BExpr
                };
                self.context.push(context);
                self.expr_allowed = true;
            }
            Tok::DollarBraceL => {
                self.context.push(Ctx::BTmpl);
                self.expr_allowed = true;
            }
            Tok::ParenL => {
                let statement_parens =
                    matches!(prev, Tok::KwIf | Tok::KwFor | Tok::KwWith | Tok::KwWhile);
                self.context.push(if statement_parens {
                    Ctx::PStat
                } else {
                    Ctx::PExpr
                });
                self.expr_allowed = true;
            }
            // Leaves `exprAllowed` as it is.
            Tok::IncDec => {}
            Tok::KwFunction | Tok::KwClass => {
                let is_expression = prev.before_expr()
                    && prev != Tok::KwElse
                    && !(prev == Tok::Semi && self.cur_context() != Some(Ctx::PStat))
                    && !(prev == Tok::KwReturn
                        && self.line_break_between(self.last_tok_end, self.start))
                    && !((prev == Tok::Colon || prev == Tok::BraceL)
                        && self.cur_context() == Some(Ctx::BStat));
                self.context.push(if is_expression {
                    Ctx::FExpr
                } else {
                    Ctx::FStat
                });
                self.expr_allowed = false;
            }
            Tok::Colon => {
                if self.cur_context().is_some_and(Ctx::is_function) {
                    self.context.pop();
                }
                self.expr_allowed = true;
            }
            Tok::BackQuote => {
                if self.cur_context() == Some(Ctx::QTmpl) {
                    self.context.pop();
                } else {
                    self.context.push(Ctx::QTmpl);
                }
                self.expr_allowed = false;
            }
            Tok::Star => {
                if prev == Tok::KwFunction
                    && let Some(last) = self.context.last_mut()
                {
                    *last = if *last == Ctx::FExpr {
                        Ctx::FExprGen
                    } else {
                        Ctx::FGen
                    };
                }
                self.expr_allowed = true;
            }
            Tok::Name => {
                let mut allowed = false;
                if prev != Tok::Dot
                    && ((self.value == "of" && !self.expr_allowed)
                        || (self.value == "yield" && self.in_generator_context()))
                {
                    allowed = true;
                }
                self.expr_allowed = allowed;
            }
            _ => self.expr_allowed = ty.before_expr(),
        }
    }

    pub(crate) fn override_context(&mut self, context: Ctx) {
        if let Some(last) = self.context.last_mut() {
            *last = context;
        }
    }

    // --- tokenize.js ---

    /// Moves to the next token. `liberal` lets a keyword written with
    /// an escape pass, where it is read as a plain name.
    pub(crate) fn next_liberal(&mut self, liberal: bool) -> PResult<()> {
        if !liberal
            && self.contains_esc
            && let Some(keyword) = self.ty.keyword()
        {
            return self.raise(self.start, format!("Escape sequence in keyword {keyword}"));
        }
        self.last_tok_end = self.end;
        self.last_tok_start = self.start;
        self.next_token()
    }

    pub(crate) fn next(&mut self) -> PResult<()> {
        self.next_liberal(false)
    }

    pub(crate) fn next_token(&mut self) -> PResult<()> {
        let context = self.cur_context();
        if context != Some(Ctx::QTmpl) {
            self.skip_space()?;
        }
        self.start = self.pos;
        if self.pos >= self.input.len() {
            self.finish_token(Tok::Eof, String::new());
            return Ok(());
        }
        if context == Some(Ctx::QTmpl) {
            return self.try_read_template_token();
        }
        let code = self.full_char_code_at(self.pos);
        self.read_token(code)
    }

    fn read_token(&mut self, code: i32) -> PResult<()> {
        // acorn-typescript: where an expression may start, `<` is one
        // token by itself (it opens type parameters or an assertion),
        // never `<<` or `<=`.
        if self.ts
            && !self.in_type
            && code == 60
            && self.expr_allowed
            && self.code_at(self.pos + 1) != 33
        {
            self.pos += 1;
            self.finish_token(Tok::Relational, "<".to_string());
            return Ok(());
        }
        if is_identifier_start(code) || code == 92 {
            return self.read_word();
        }
        self.get_token_from_code(code)
    }

    pub(crate) fn full_char_code_at(&self, pos: usize) -> i32 {
        let code = self.code_at(pos);
        if code <= 0xd7ff || code >= 0xdc00 {
            return code;
        }
        let next = self.code_at(pos + 1);
        if next <= 0xdbff || next >= 0xe000 {
            code
        } else {
            (code << 10) + next - 0x35f_dc00
        }
    }

    fn skip_block_comment(&mut self) -> PResult<()> {
        self.pos += 2;
        let mut end = self.pos;
        loop {
            if end + 1 >= self.input.len() {
                return self.raise(self.pos - 2, "Unterminated comment");
            }
            if self.input[end] == 42 && self.input[end + 1] == 47 {
                break;
            }
            end += 1;
        }
        self.pos = end + 2;
        Ok(())
    }

    pub(crate) fn skip_line_comment(&mut self, start_skip: usize) {
        self.pos += start_skip;
        while self.pos < self.input.len() && !is_new_line(self.code_at(self.pos)) {
            self.pos += 1;
        }
    }

    fn skip_space(&mut self) -> PResult<()> {
        while self.pos < self.input.len() {
            let ch = self.code_at(self.pos);
            match ch {
                32 | 160 => self.pos += 1,
                13 => {
                    if self.code_at(self.pos + 1) == 10 {
                        self.pos += 1;
                    }
                    self.pos += 1;
                }
                10 | 8232 | 8233 => self.pos += 1,
                47 => match self.code_at(self.pos + 1) {
                    42 => self.skip_block_comment()?,
                    47 => self.skip_line_comment(2),
                    _ => break,
                },
                _ => {
                    let non_ascii_space = matches!(
                        ch,
                        0x1680 | 0x2000..=0x200a | 0x202f | 0x205f | 0x3000 | 0xfeff
                    );
                    if (ch > 8 && ch < 14) || non_ascii_space {
                        self.pos += 1;
                    } else {
                        break;
                    }
                }
            }
        }
        Ok(())
    }

    fn finish_token(&mut self, ty: Tok, value: String) {
        self.end = self.pos;
        let prev = self.ty;
        self.ty = ty;
        self.value = value;
        // acorn-typescript reads ahead without the tokens' effect on
        // the context.
        if !self.is_lookahead {
            self.update_context(prev);
        }
    }

    fn finish_punctuation(&mut self, ty: Tok) {
        self.pos += 1;
        self.finish_token(ty, String::new());
    }

    pub(crate) fn finish_op(&mut self, ty: Tok, size: usize) {
        let text = self.slice(self.pos, self.pos + size);
        self.pos += size;
        self.finish_token(ty, text);
    }

    fn read_token_dot(&mut self) -> PResult<()> {
        let next = self.code_at(self.pos + 1);
        if (48..=57).contains(&next) {
            return self.read_number(true);
        }
        let next2 = self.code_at(self.pos + 2);
        if next == 46 && next2 == 46 {
            self.pos += 3;
            self.finish_token(Tok::Ellipsis, String::new());
        } else {
            self.pos += 1;
            self.finish_token(Tok::Dot, String::new());
        }
        Ok(())
    }

    fn read_token_slash(&mut self) -> PResult<()> {
        let next = self.code_at(self.pos + 1);
        if self.expr_allowed {
            self.pos += 1;
            return self.read_regexp();
        }
        if next == 61 {
            self.finish_op(Tok::Assign, 2);
        } else {
            self.finish_op(Tok::Slash, 1);
        }
        Ok(())
    }

    fn read_token_mult_modulo_exp(&mut self, code: i32) {
        let mut next = self.code_at(self.pos + 1);
        let mut size = 1;
        let mut ty = if code == 42 { Tok::Star } else { Tok::Modulo };
        if code == 42 && next == 42 {
            size += 1;
            ty = Tok::StarStar;
            next = self.code_at(self.pos + 2);
        }
        if next == 61 {
            self.finish_op(Tok::Assign, size + 1);
        } else {
            self.finish_op(ty, size);
        }
    }

    fn read_token_pipe_amp(&mut self, code: i32) {
        let next = self.code_at(self.pos + 1);
        if next == code {
            if self.code_at(self.pos + 2) == 61 {
                return self.finish_op(Tok::Assign, 3);
            }
            let ty = if code == 124 {
                Tok::LogicalOr
            } else {
                Tok::LogicalAnd
            };
            return self.finish_op(ty, 2);
        }
        if next == 61 {
            return self.finish_op(Tok::Assign, 2);
        }
        let ty = if code == 124 {
            Tok::BitwiseOr
        } else {
            Tok::BitwiseAnd
        };
        self.finish_op(ty, 1);
    }

    fn read_token_plus_min(&mut self, code: i32) {
        let next = self.code_at(self.pos + 1);
        if next == code {
            // The `-->` comment of a script is no comment in a module.
            return self.finish_op(Tok::IncDec, 2);
        }
        if next == 61 {
            return self.finish_op(Tok::Assign, 2);
        }
        self.finish_op(Tok::PlusMin, 1);
    }

    pub(crate) fn read_token_lt_gt(&mut self, code: i32) {
        let next = self.code_at(self.pos + 1);
        let mut size = 1;
        if next == code {
            size = if code == 62 && self.code_at(self.pos + 2) == 62 {
                3
            } else {
                2
            };
            if self.code_at(self.pos + size) == 61 {
                return self.finish_op(Tok::Assign, size + 1);
            }
            return self.finish_op(Tok::BitShift, size);
        }
        // The `<!--` comment of a script is no comment in a module.
        if next == 61 {
            size = 2;
        }
        self.finish_op(Tok::Relational, size);
    }

    fn read_token_eq_excl(&mut self, code: i32) {
        let next = self.code_at(self.pos + 1);
        if next == 61 {
            let size = if self.code_at(self.pos + 2) == 61 {
                3
            } else {
                2
            };
            return self.finish_op(Tok::Equality, size);
        }
        if code == 61 && next == 62 {
            self.pos += 2;
            return self.finish_token(Tok::Arrow, String::new());
        }
        self.finish_op(if code == 61 { Tok::Eq } else { Tok::Prefix }, 1);
    }

    fn read_token_question(&mut self) {
        let next = self.code_at(self.pos + 1);
        if next == 46 {
            // Past the end of the input neither comparison holds.
            let next2 = self.code_at(self.pos + 2);
            if next2 >= 0 && !(48..=57).contains(&next2) {
                return self.finish_op(Tok::QuestionDot, 2);
            }
        }
        if next == 63 {
            if self.code_at(self.pos + 2) == 61 {
                return self.finish_op(Tok::Assign, 3);
            }
            return self.finish_op(Tok::Coalesce, 2);
        }
        self.finish_op(Tok::Question, 1);
    }

    fn read_token_number_sign(&mut self) -> PResult<()> {
        self.pos += 1;
        let code = self.full_char_code_at(self.pos);
        if is_identifier_start(code) || code == 92 {
            let word = self.read_word1()?;
            self.finish_token(Tok::PrivateId, word);
            return Ok(());
        }
        self.raise(
            self.pos,
            format!("Unexpected character '{}'", unexpected_character(code)),
        )
    }

    fn get_token_from_code(&mut self, code: i32) -> PResult<()> {
        if self.ts {
            // U+0085 (next line) is white space to TypeScript, which
            // acorn does not skip: it is stepped over here.
            if code == 133 {
                self.pos += 1;
                return self.next_token();
            }
            // In a type every `<` and `>` stands alone.
            if self.in_type && (code == 62 || code == 60) {
                self.finish_op(Tok::Relational, 1);
                return Ok(());
            }
            if code == 64 {
                self.pos += 1;
                self.finish_token(Tok::At, String::new());
                return Ok(());
            }
        }
        match code {
            46 => return self.read_token_dot(),
            40 => self.finish_punctuation(Tok::ParenL),
            41 => self.finish_punctuation(Tok::ParenR),
            59 => self.finish_punctuation(Tok::Semi),
            44 => self.finish_punctuation(Tok::Comma),
            91 => self.finish_punctuation(Tok::BracketL),
            93 => self.finish_punctuation(Tok::BracketR),
            123 => self.finish_punctuation(Tok::BraceL),
            125 => self.finish_punctuation(Tok::BraceR),
            58 => self.finish_punctuation(Tok::Colon),
            96 => self.finish_punctuation(Tok::BackQuote),
            48 => {
                let next = self.code_at(self.pos + 1);
                return match next {
                    120 | 88 => self.read_radix_number(16),
                    111 | 79 => self.read_radix_number(8),
                    98 | 66 => self.read_radix_number(2),
                    _ => self.read_number(false),
                };
            }
            49..=57 => return self.read_number(false),
            34 | 39 => return self.read_string(code),
            47 => return self.read_token_slash(),
            37 | 42 => self.read_token_mult_modulo_exp(code),
            124 | 38 => self.read_token_pipe_amp(code),
            94 => {
                if self.code_at(self.pos + 1) == 61 {
                    self.finish_op(Tok::Assign, 2);
                } else {
                    self.finish_op(Tok::BitwiseXor, 1);
                }
            }
            43 | 45 => self.read_token_plus_min(code),
            60 | 62 => self.read_token_lt_gt(code),
            61 | 33 => self.read_token_eq_excl(code),
            63 => self.read_token_question(),
            126 => self.finish_op(Tok::Prefix, 1),
            35 => return self.read_token_number_sign(),
            _ => {
                return self.raise(
                    self.pos,
                    format!("Unexpected character '{}'", code_point_to_string(code)),
                );
            }
        }
        Ok(())
    }

    /// Reads the regular expression whose `/` the tokenizer has just
    /// passed, or that the parser found where it wanted an expression.
    pub(crate) fn read_regexp(&mut self) -> PResult<()> {
        let start = self.pos;
        let mut escaped = false;
        let mut in_class = false;
        loop {
            if self.pos >= self.input.len() {
                return self.raise(start, "Unterminated regular expression");
            }
            let ch = self.code_at(self.pos);
            if is_new_line(ch) {
                return self.raise(start, "Unterminated regular expression");
            }
            if escaped {
                escaped = false;
            } else {
                if ch == 91 {
                    in_class = true;
                } else if ch == 93 && in_class {
                    in_class = false;
                } else if ch == 47 && !in_class {
                    break;
                }
                escaped = ch == 92;
            }
            self.pos += 1;
        }
        let pattern = self.input[start..self.pos].to_vec();
        self.pos += 1;
        let flags_start = self.pos;
        let flags = self.read_word1()?;
        if self.contains_esc {
            return self.unexpected_at(flags_start);
        }

        // validateRegExpFlags
        let mut seen_u = false;
        let mut seen_v = false;
        for (i, flag) in flags.char_indices() {
            if !"gimuysdv".contains(flag) {
                return self.raise(start, "Invalid regular expression flag");
            }
            if flags[i + flag.len_utf8()..].contains(flag) {
                return self.raise(start, "Duplicate regular expression flag");
            }
            seen_u |= flag == 'u';
            seen_v |= flag == 'v';
        }
        if seen_u && seen_v {
            return self.raise(start, "Invalid regular expression flag");
        }
        if let Some(reason) = regexp::pattern_error(&pattern, &flags) {
            if reason == regexp::STACK_EXHAUSTED {
                return self.stack_exhausted();
            }
            let source = String::from_utf16_lossy(&pattern);
            return self.raise(
                start,
                format!("Invalid regular expression: /{source}/: {reason}"),
            );
        }
        self.finish_token(Tok::Regexp, String::new());
        Ok(())
    }

    /// `readInt`: the digits' value, or `None` when there are none (or
    /// not exactly `len` of them). `len` set means an escape sequence,
    /// which takes no separators.
    fn read_int(
        &mut self,
        radix: i32,
        len: Option<i64>,
        maybe_legacy_octal: bool,
    ) -> PResult<Option<f64>> {
        let allow_separators = len.is_none();
        let legacy_octal = maybe_legacy_octal && self.code_at(self.pos) == 48;
        let start = self.pos;
        let mut total = 0.0;
        let mut last_code = 0;
        let mut i: i64 = 0;
        while len.is_none_or(|len| i < len) {
            let code = self.code_at(self.pos);
            if allow_separators && code == 95 {
                if legacy_octal {
                    return self.raise(
                        self.pos,
                        "Numeric separator is not allowed in legacy octal numeric literals",
                    );
                }
                if last_code == 95 {
                    return self
                        .raise(self.pos, "Numeric separator must be exactly one underscore");
                }
                if i == 0 {
                    return self.raise(
                        self.pos,
                        "Numeric separator is not allowed at the first of digits",
                    );
                }
                last_code = code;
                i += 1;
                self.pos += 1;
                continue;
            }
            let val = if code >= 97 {
                code - 97 + 10
            } else if code >= 65 {
                code - 65 + 10
            } else if (48..=57).contains(&code) {
                code - 48
            } else {
                i32::MAX
            };
            if val >= radix {
                break;
            }
            last_code = code;
            total = total * f64::from(radix) + f64::from(val);
            i += 1;
            self.pos += 1;
        }
        if allow_separators && last_code == 95 {
            return self.raise(
                self.pos - 1,
                "Numeric separator is not allowed at the last of digits",
            );
        }
        if self.pos == start || len.is_some_and(|len| (self.pos - start) as i64 != len) {
            return Ok(None);
        }
        Ok(Some(total))
    }

    fn read_radix_number(&mut self, radix: i32) -> PResult<()> {
        self.pos += 2;
        if self.read_int(radix, None, false)?.is_none() {
            return self.raise(self.start + 2, format!("Expected number in radix {radix}"));
        }
        if self.code_at(self.pos) == 110 {
            self.pos += 1;
        } else if is_identifier_start(self.full_char_code_at(self.pos)) {
            return self.raise(self.pos, "Identifier directly after number");
        }
        self.finish_token(Tok::Num, String::new());
        Ok(())
    }

    fn read_number(&mut self, starts_with_dot: bool) -> PResult<()> {
        let start = self.pos;
        if !starts_with_dot && self.read_int(10, None, true)?.is_none() {
            return self.raise(start, "Invalid number");
        }
        let octal = self.pos - start >= 2 && self.code_at(start) == 48;
        if octal && self.strict {
            return self.raise(start, "Invalid number");
        }
        let mut next = self.code_at(self.pos);
        if !octal && !starts_with_dot && next == 110 {
            self.pos += 1;
            if is_identifier_start(self.full_char_code_at(self.pos)) {
                return self.raise(self.pos, "Identifier directly after number");
            }
            self.finish_token(Tok::Num, String::new());
            return Ok(());
        }
        // A legacy octal literal with an 8 or 9 in it is decimal.
        let octal = octal
            && !self.input[start..self.pos]
                .iter()
                .any(|&c| c == 56 || c == 57);
        if next == 46 && !octal {
            self.pos += 1;
            self.read_int(10, None, false)?;
            next = self.code_at(self.pos);
        }
        if (next == 69 || next == 101) && !octal {
            self.pos += 1;
            next = self.code_at(self.pos);
            if next == 43 || next == 45 {
                self.pos += 1;
            }
            if self.read_int(10, None, false)?.is_none() {
                return self.raise(start, "Invalid number");
            }
        }
        if is_identifier_start(self.full_char_code_at(self.pos)) {
            return self.raise(self.pos, "Identifier directly after number");
        }
        self.finish_token(Tok::Num, String::new());
        Ok(())
    }

    fn read_code_point(&mut self) -> PResult<f64> {
        let ch = self.code_at(self.pos);
        if ch == 123 {
            self.pos += 1;
            let code_pos = self.pos;
            let close = self.input[self.pos.min(self.input.len())..]
                .iter()
                .position(|&c| c == 125)
                .map_or(-1, |i| (self.pos + i) as i64);
            let code = self.read_hex_char(close - self.pos as i64)?;
            self.pos += 1;
            if code > f64::from(0x10_ffff) {
                return self.invalid_string_token(code_pos, "Code point out of bounds");
            }
            Ok(code)
        } else {
            self.read_hex_char(4)
        }
    }

    fn read_string(&mut self, quote: i32) -> PResult<()> {
        let mut out: Vec<u16> = Vec::new();
        self.pos += 1;
        let mut chunk_start = self.pos;
        loop {
            if self.pos >= self.input.len() {
                return self.raise(self.start, "Unterminated string constant");
            }
            let ch = self.code_at(self.pos);
            if ch == quote {
                break;
            }
            if ch == 92 {
                out.extend_from_slice(&self.input[chunk_start..self.pos]);
                self.read_escaped_char(false, &mut out)?;
                chunk_start = self.pos;
            } else if ch == 0x2028 || ch == 0x2029 {
                self.pos += 1;
            } else {
                if is_new_line(ch) {
                    return self.raise(self.start, "Unterminated string constant");
                }
                self.pos += 1;
            }
        }
        let chunk_end = self.pos.min(self.input.len());
        out.extend_from_slice(&self.input[chunk_start.min(chunk_end)..chunk_end]);
        self.pos += 1;
        self.value_lone_surrogate = has_lone_surrogate(&out);
        self.value_units = if self.value_lone_surrogate {
            out.clone()
        } else {
            Vec::new()
        };
        self.finish_token(Tok::String, String::from_utf16_lossy(&out));
        Ok(())
    }

    fn try_read_template_token(&mut self) -> PResult<()> {
        self.in_template_element = true;
        let result = match self.read_tmpl_token() {
            Err(Stop::InvalidTemplateEscape) => self.read_invalid_template_token(),
            other => other,
        };
        self.in_template_element = false;
        result
    }

    fn invalid_string_token<T>(&self, position: usize, message: &str) -> PResult<T> {
        if self.in_template_element {
            Err(Stop::InvalidTemplateEscape)
        } else {
            self.raise(position, message)
        }
    }

    fn read_tmpl_token(&mut self) -> PResult<()> {
        let mut scratch: Vec<u16> = Vec::new();
        loop {
            if self.pos >= self.input.len() {
                return self.raise(self.start, "Unterminated template");
            }
            let ch = self.code_at(self.pos);
            if ch == 96 || (ch == 36 && self.code_at(self.pos + 1) == 123) {
                if self.pos == self.start
                    && (self.ty == Tok::Template || self.ty == Tok::InvalidTemplate)
                {
                    if ch == 36 {
                        self.pos += 2;
                        self.finish_token(Tok::DollarBraceL, String::new());
                    } else {
                        self.pos += 1;
                        self.finish_token(Tok::BackQuote, String::new());
                    }
                    return Ok(());
                }
                self.finish_token(Tok::Template, String::new());
                return Ok(());
            }
            if ch == 92 {
                self.read_escaped_char(true, &mut scratch)?;
            } else if is_new_line(ch) {
                self.pos += 1;
                if ch == 13 && self.code_at(self.pos) == 10 {
                    self.pos += 1;
                }
            } else {
                self.pos += 1;
            }
        }
    }

    fn read_invalid_template_token(&mut self) -> PResult<()> {
        while self.pos < self.input.len() {
            match self.code_at(self.pos) {
                92 => self.pos += 1,
                36 if self.code_at(self.pos + 1) != 123 => {}
                36 | 96 => {
                    self.finish_token(Tok::InvalidTemplate, String::new());
                    return Ok(());
                }
                _ => {}
            }
            self.pos += 1;
        }
        self.raise(self.start, "Unterminated template")
    }

    /// Reads the escape at the `\` the tokenizer stands on, adding what
    /// it stands for to `out`.
    fn read_escaped_char(&mut self, in_template: bool, out: &mut Vec<u16>) -> PResult<()> {
        self.pos += 1;
        let mut ch = self.code_at(self.pos);
        self.pos += 1;
        match ch {
            110 => out.push(10),
            114 => out.push(13),
            120 => {
                let code = self.read_hex_char(2)?;
                out.push(code as u16);
            }
            117 => {
                let code = self.read_code_point()?;
                code_point_to_units(code as u32, out);
            }
            116 => out.push(9),
            98 => out.push(8),
            118 => out.push(11),
            102 => out.push(12),
            13 => {
                if self.code_at(self.pos) == 10 {
                    self.pos += 1;
                }
            }
            10 => {}
            _ => {
                if ch == 56 || ch == 57 {
                    if self.strict {
                        return self.invalid_string_token(self.pos - 1, "Invalid escape sequence");
                    }
                    if in_template {
                        return self.invalid_string_token(
                            self.pos - 1,
                            "Invalid escape sequence in template string",
                        );
                    }
                }
                if (48..=55).contains(&ch) {
                    // Up to three octal digits, as many as stay below 256.
                    let mut digits = 0;
                    while digits < 3 && (48..=55).contains(&self.code_at(self.pos - 1 + digits)) {
                        digits += 1;
                    }
                    let value_of = |parser: &Parser, digits: usize| {
                        (0..digits).fold(0, |value, i| {
                            value * 8 + (parser.code_at(parser.pos - 1 + i) - 48)
                        })
                    };
                    let mut octal = value_of(self, digits);
                    if octal > 255 {
                        digits -= 1;
                        octal = value_of(self, digits);
                    }
                    let is_zero = digits == 1 && ch == 48;
                    self.pos += digits - 1;
                    ch = self.code_at(self.pos);
                    if (!is_zero || ch == 56 || ch == 57) && (self.strict || in_template) {
                        return self.invalid_string_token(
                            self.pos - 1 - digits,
                            if in_template {
                                "Octal literal in template string"
                            } else {
                                "Octal literal in strict mode"
                            },
                        );
                    }
                    out.push(octal as u16);
                } else if !is_new_line(ch) {
                    // Past the end of the input this is `\0`.
                    out.push(ch.max(0) as u16);
                }
            }
        }
        Ok(())
    }

    fn read_hex_char(&mut self, len: i64) -> PResult<f64> {
        let code_pos = self.pos;
        match self.read_int(16, Some(len), false)? {
            Some(n) => Ok(n),
            None => self.invalid_string_token(code_pos, "Bad character escape sequence"),
        }
    }

    /// Reads a word, escapes decoded. Sets `contains_esc`.
    pub(crate) fn read_word1(&mut self) -> PResult<String> {
        self.contains_esc = false;
        let mut word: Vec<u16> = Vec::new();
        let mut first = true;
        let mut chunk_start = self.pos;
        while self.pos < self.input.len() {
            let ch = self.full_char_code_at(self.pos);
            if is_identifier_char(ch) {
                self.pos += if ch <= 0xffff { 1 } else { 2 };
            } else if ch == 92 {
                self.contains_esc = true;
                word.extend_from_slice(&self.input[chunk_start..self.pos]);
                let esc_start = self.pos;
                self.pos += 1;
                if self.code_at(self.pos) != 117 {
                    return self.invalid_string_token(
                        self.pos,
                        "Expecting Unicode escape sequence \\uXXXX",
                    );
                }
                self.pos += 1;
                let esc = self.read_code_point()? as i32;
                let fits = if first {
                    is_identifier_start(esc)
                } else {
                    is_identifier_char(esc)
                };
                if !fits {
                    return self.invalid_string_token(esc_start, "Invalid Unicode escape");
                }
                code_point_to_units(esc as u32, &mut word);
                chunk_start = self.pos;
            } else {
                break;
            }
            first = false;
        }
        let end = self.pos.min(self.input.len());
        word.extend_from_slice(&self.input[chunk_start.min(end)..end]);
        Ok(String::from_utf16_lossy(&word))
    }

    fn read_word(&mut self) -> PResult<()> {
        let word = self.read_word1()?;
        let ty = Tok::of_keyword(&word)
            .or_else(|| self.ts.then(|| Tok::of_ts_word(&word)).flatten())
            .unwrap_or(Tok::Name);
        self.finish_token(ty, word);
        Ok(())
    }
}

/// The character acorn names in `Unexpected character '…'` after a `#`.
/// At the end of the input its arithmetic on "no character" comes out
/// as U+10000.
fn unexpected_character(code: i32) -> String {
    if code < 0 {
        "\u{10000}".to_string()
    } else {
        code_point_to_string(code)
    }
}
