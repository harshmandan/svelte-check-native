//! acorn's check of a regular expression pattern.
//!
//! acorn validates a regular expression literal itself, as soon as its
//! tokenizer has read it, and throws at the first problem with a
//! message worded after V8's. What it accepts and how it words a
//! rejection both differ from other validators (it lets `{9…9}` grow
//! past any integer, and tells "Incomplete quantifier" from "Lone
//! quantifier brackets"), so this is its `RegExpValidationState` and
//! `regexp_*` methods carried over one for one, for the ECMAScript
//! version the Svelte compiler asks for (2025).
//!
//! A pattern is read as UTF-16 code units, as JavaScript strings are:
//! without the `u` or `v` flag a character outside the basic plane is
//! two units, with it one code point.

mod properties;

use std::collections::HashMap;

/// acorn's reason for rejecting `pattern` under `flags`, or `None` when
/// it accepts it. The flags themselves are taken as valid.
pub(crate) fn pattern_error(pattern: &[u16], flags: &str) -> Option<&'static str> {
    let unicode_sets = flags.contains('v');
    let unicode = flags.contains('u');
    let mut state = State {
        source: pattern.to_vec(),
        switch_u: unicode_sets || unicode,
        switch_v: unicode_sets,
        switch_n: unicode_sets || unicode,
        pos: 0,
        last_int_value: 0.0,
        last_string_value: String::new(),
        last_assertion_is_quantifiable: false,
        num_capturing_parens: 0.0,
        max_back_reference: 0.0,
        group_names: HashMap::new(),
        back_reference_names: Vec::new(),
        branches: Vec::new(),
        branch: None,
        depth: 0,
    };
    state.validate().err()
}

/// The reason given for a pattern nested deeper than acorn has stack
/// for. It is no fault of the pattern: acorn reports it without the
/// `Invalid regular expression` wording, at the literal's start.
pub(crate) const STACK_EXHAUSTED: &str = "Not enough stack space to parse input";

/// About how deep groups nest before acorn runs out of stack in Node 24.
const MAX_GROUP_DEPTH: u32 = 2500;

/// A reason to reject the pattern.
type Raised = &'static str;

/// What a class construct turned out to be.
#[derive(Clone, Copy, PartialEq, Eq)]
enum CharSet {
    /// Nothing was read.
    None,
    /// Read, and it cannot match strings.
    Ok,
    /// Read, and it can match strings.
    String,
}

impl CharSet {
    fn read(self) -> bool {
        self != CharSet::None
    }
}

/// One branch of a disjunction. Two groups may share a name when they
/// lie in separate branches.
#[derive(Clone, Copy)]
struct Branch {
    /// The branch the disjunction itself lies in.
    parent: Option<usize>,
    /// The first branch of the disjunction: its siblings share it.
    base: usize,
}

struct State {
    source: Vec<u16>,
    switch_u: bool,
    switch_v: bool,
    switch_n: bool,
    pos: usize,
    /// A JavaScript number: digits accumulate past any integer's range.
    last_int_value: f64,
    last_string_value: String,
    last_assertion_is_quantifiable: bool,
    num_capturing_parens: f64,
    max_back_reference: f64,
    group_names: HashMap<String, Vec<Option<usize>>>,
    back_reference_names: Vec<String>,
    branches: Vec<Branch>,
    branch: Option<usize>,
    /// How deep groups and classes are nested.
    depth: u32,
}

const END: i32 = -1;

fn ch(c: char) -> i32 {
    c as i32
}

fn is_syntax_character(c: i32) -> bool {
    c == ch('$')
        || (ch('(')..=ch('+')).contains(&c)
        || c == ch('.')
        || c == ch('?')
        || (ch('[')..=ch('^')).contains(&c)
        || (ch('{')..=ch('}')).contains(&c)
}

fn is_regular_expression_modifier(c: i32) -> bool {
    c == ch('i') || c == ch('m') || c == ch('s')
}

fn as_char(c: i32) -> Option<char> {
    u32::try_from(c).ok().and_then(char::from_u32)
}

fn is_identifier_start(c: i32) -> bool {
    c == ch('$') || c == ch('_') || crate::identifier::is_identifier_start(c)
}

fn is_identifier_part(c: i32) -> bool {
    c == ch('$')
        || c == ch('_')
        || c == 0x200C
        || c == 0x200D
        || crate::identifier::is_identifier_char(c)
}

fn is_control_letter(c: i32) -> bool {
    (ch('A')..=ch('Z')).contains(&c) || (ch('a')..=ch('z')).contains(&c)
}

fn is_character_class_escape(c: i32) -> bool {
    [ch('d'), ch('D'), ch('s'), ch('S'), ch('w'), ch('W')].contains(&c)
}

fn is_unicode_property_name_character(c: i32) -> bool {
    is_control_letter(c) || c == ch('_')
}

fn is_unicode_property_value_character(c: i32) -> bool {
    is_unicode_property_name_character(c) || is_decimal_digit(c)
}

fn is_class_set_reserved_double_punctuator_character(c: i32) -> bool {
    c == ch('!')
        || (ch('#')..=ch('&')).contains(&c)
        || (ch('*')..=ch(',')).contains(&c)
        || c == ch('.')
        || (ch(':')..=ch('@')).contains(&c)
        || c == ch('^')
        || c == ch('`')
        || c == ch('~')
}

fn is_class_set_syntax_character(c: i32) -> bool {
    c == ch('(')
        || c == ch(')')
        || c == ch('-')
        || c == ch('/')
        || (ch('[')..=ch(']')).contains(&c)
        || (ch('{')..=ch('}')).contains(&c)
}

fn is_class_set_reserved_punctuator(c: i32) -> bool {
    c == ch('!')
        || c == ch('#')
        || c == ch('%')
        || c == ch('&')
        || c == ch(',')
        || c == ch('-')
        || (ch(':')..=ch('>')).contains(&c)
        || c == ch('@')
        || c == ch('`')
        || c == ch('~')
}

fn is_decimal_digit(c: i32) -> bool {
    (ch('0')..=ch('9')).contains(&c)
}

fn is_hex_digit(c: i32) -> bool {
    is_decimal_digit(c) || (ch('A')..=ch('F')).contains(&c) || (ch('a')..=ch('f')).contains(&c)
}

fn hex_to_int(c: i32) -> i32 {
    if (ch('A')..=ch('F')).contains(&c) {
        10 + (c - ch('A'))
    } else if (ch('a')..=ch('f')).contains(&c) {
        10 + (c - ch('a'))
    } else {
        c - ch('0')
    }
}

fn is_octal_digit(c: i32) -> bool {
    (ch('0')..=ch('7')).contains(&c)
}

fn is_valid_unicode(value: f64) -> bool {
    (0.0..=f64::from(0x10FFFF)).contains(&value)
}

fn listed(words: &[&str], word: &str) -> bool {
    words.binary_search(&word).is_ok()
}

/// The values a `Name=Value` property takes, when the name is one.
fn non_binary_values(name: &str) -> Option<&'static [&'static str]> {
    match name {
        "General_Category" | "gc" => Some(properties::GENERAL_CATEGORY),
        "Script" | "sc" | "Script_Extensions" | "scx" => Some(properties::SCRIPT),
        _ => None,
    }
}

impl State {
    /// The code point at `i` with the `u` flag (or `force_u`), else the
    /// code unit there.
    fn at(&self, i: usize, force_u: bool) -> i32 {
        let Some(&c) = self.source.get(i) else {
            return END;
        };
        let c = i32::from(c);
        if !(force_u || self.switch_u) || c <= 0xD7FF || c >= 0xE000 {
            return c;
        }
        match self.source.get(i + 1) {
            Some(&next) if (0xDC00..=0xDFFF).contains(&next) => {
                (c << 10) + i32::from(next) - 0x35F_DC00
            }
            _ => c,
        }
    }

    fn next_index(&self, i: usize, force_u: bool) -> usize {
        let Some(&c) = self.source.get(i) else {
            return self.source.len();
        };
        if !(force_u || self.switch_u) || c <= 0xD7FF || c >= 0xE000 {
            return i + 1;
        }
        match self.source.get(i + 1) {
            Some(next) if (0xDC00..=0xDFFF).contains(next) => i + 2,
            _ => i + 1,
        }
    }

    fn current(&self) -> i32 {
        self.at(self.pos, false)
    }

    fn lookahead(&self) -> i32 {
        self.at(self.next_index(self.pos, false), false)
    }

    fn advance(&mut self) {
        self.pos = self.next_index(self.pos, false);
    }

    fn eat(&mut self, c: char) -> bool {
        if self.current() == ch(c) {
            self.advance();
            return true;
        }
        false
    }

    fn eat_chars(&mut self, chars: [char; 2]) -> bool {
        let mut pos = self.pos;
        for c in chars {
            if self.at(pos, false) != ch(c) {
                return false;
            }
            pos = self.next_index(pos, false);
        }
        self.pos = pos;
        true
    }

    /// `validateRegExpPattern`.
    fn validate(&mut self) -> Result<(), Raised> {
        self.pattern()?;
        // A pattern with a named group is read again with `\k` as a
        // named reference.
        if !self.switch_n && !self.group_names.is_empty() {
            self.switch_n = true;
            self.pattern()?;
        }
        Ok(())
    }

    fn pattern(&mut self) -> Result<(), Raised> {
        self.pos = 0;
        self.last_int_value = 0.0;
        self.last_string_value.clear();
        self.last_assertion_is_quantifiable = false;
        self.num_capturing_parens = 0.0;
        self.max_back_reference = 0.0;
        self.group_names.clear();
        self.back_reference_names.clear();
        self.branches.clear();
        self.branch = None;

        self.disjunction()?;

        if self.pos != self.source.len() {
            if self.eat(')') {
                return Err("Unmatched ')'");
            }
            if self.eat(']') || self.eat('}') {
                return Err("Lone quantifier brackets");
            }
        }
        if self.max_back_reference > self.num_capturing_parens {
            return Err("Invalid escape");
        }
        for name in &self.back_reference_names {
            if !self.group_names.contains_key(name) {
                return Err("Invalid named capture referenced");
            }
        }
        Ok(())
    }

    /// Runs one level of group or class nesting with room on the stack.
    fn nested<T>(&mut self, f: impl FnOnce(&mut Self) -> Result<T, Raised>) -> Result<T, Raised> {
        self.depth += 1;
        let result = if self.depth > MAX_GROUP_DEPTH {
            Err(STACK_EXHAUSTED)
        } else {
            crate::state::with_stack(|| f(self))
        };
        self.depth -= 1;
        result
    }

    fn disjunction(&mut self) -> Result<(), Raised> {
        self.nested(Self::disjunction_inner)
    }

    fn disjunction_inner(&mut self) -> Result<(), Raised> {
        let first = self.branches.len();
        self.branches.push(Branch {
            parent: self.branch,
            base: first,
        });
        self.branch = Some(first);
        self.alternative()?;
        while self.eat('|') {
            let sibling = self.branches.len();
            self.branches.push(self.branches[first]);
            self.branch = Some(sibling);
            self.alternative()?;
        }
        self.branch = self.branches[first].parent;

        if self.eat_quantifier(true)? {
            return Err("Nothing to repeat");
        }
        if self.eat('{') {
            return Err("Lone quantifier brackets");
        }
        Ok(())
    }

    /// Whether the branches `a` and `b`, or any they lie in, are
    /// different branches of one disjunction.
    fn separated(&self, a: Option<usize>, b: Option<usize>) -> bool {
        let mut own = a;
        while let Some(i) = own {
            let mut other = b;
            while let Some(j) = other {
                if self.branches[i].base == self.branches[j].base && i != j {
                    return true;
                }
                other = self.branches[j].parent;
            }
            own = self.branches[i].parent;
        }
        false
    }

    fn alternative(&mut self) -> Result<(), Raised> {
        while self.pos < self.source.len() && self.eat_term()? {}
        Ok(())
    }

    fn eat_term(&mut self) -> Result<bool, Raised> {
        if self.eat_assertion()? {
            if self.last_assertion_is_quantifiable && self.eat_quantifier(false)? && self.switch_u {
                return Err("Invalid quantifier");
            }
            return Ok(true);
        }
        let atom = if self.switch_u {
            self.eat_atom()?
        } else {
            self.eat_extended_atom()?
        };
        if atom {
            self.eat_quantifier(false)?;
            return Ok(true);
        }
        Ok(false)
    }

    fn eat_assertion(&mut self) -> Result<bool, Raised> {
        let start = self.pos;
        self.last_assertion_is_quantifiable = false;

        if self.eat('^') || self.eat('$') {
            return Ok(true);
        }
        if self.eat('\\') {
            if self.eat('B') || self.eat('b') {
                return Ok(true);
            }
            self.pos = start;
        }
        if self.eat('(') && self.eat('?') {
            let lookbehind = self.eat('<');
            if self.eat('=') || self.eat('!') {
                self.disjunction()?;
                if !self.eat(')') {
                    return Err("Unterminated group");
                }
                self.last_assertion_is_quantifiable = !lookbehind;
                return Ok(true);
            }
        }
        self.pos = start;
        Ok(false)
    }

    fn eat_quantifier(&mut self, no_error: bool) -> Result<bool, Raised> {
        if self.eat_quantifier_prefix(no_error)? {
            self.eat('?');
            return Ok(true);
        }
        Ok(false)
    }

    fn eat_quantifier_prefix(&mut self, no_error: bool) -> Result<bool, Raised> {
        if self.eat('*') || self.eat('+') || self.eat('?') {
            return Ok(true);
        }
        self.eat_braced_quantifier(no_error)
    }

    fn eat_braced_quantifier(&mut self, no_error: bool) -> Result<bool, Raised> {
        let start = self.pos;
        if self.eat('{') {
            if self.eat_decimal_digits() {
                let min = self.last_int_value;
                let mut max = -1.0;
                if self.eat(',') && self.eat_decimal_digits() {
                    max = self.last_int_value;
                }
                if self.eat('}') {
                    if max != -1.0 && max < min && !no_error {
                        return Err("numbers out of order in {} quantifier");
                    }
                    return Ok(true);
                }
            }
            if self.switch_u && !no_error {
                return Err("Incomplete quantifier");
            }
            self.pos = start;
        }
        Ok(false)
    }

    fn eat_atom(&mut self) -> Result<bool, Raised> {
        Ok(self.eat_pattern_characters()
            || self.eat('.')
            || self.eat_reverse_solidus_atom_escape()?
            || self.eat_character_class()?
            || self.eat_uncapturing_group()?
            || self.eat_capturing_group()?)
    }

    fn eat_reverse_solidus_atom_escape(&mut self) -> Result<bool, Raised> {
        let start = self.pos;
        if self.eat('\\') {
            if self.eat_atom_escape()? {
                return Ok(true);
            }
            self.pos = start;
        }
        Ok(false)
    }

    fn eat_uncapturing_group(&mut self) -> Result<bool, Raised> {
        let start = self.pos;
        if self.eat('(') {
            if self.eat('?') {
                let add = self.eat_modifiers();
                let has_hyphen = self.eat('-');
                if !add.is_empty() || has_hyphen {
                    for (i, modifier) in add.iter().enumerate() {
                        if add[i + 1..].contains(modifier) {
                            return Err("Duplicate regular expression modifiers");
                        }
                    }
                    if has_hyphen {
                        let remove = self.eat_modifiers();
                        if add.is_empty() && remove.is_empty() && self.current() == ch(':') {
                            return Err("Invalid regular expression modifiers");
                        }
                        for (i, modifier) in remove.iter().enumerate() {
                            if remove[i + 1..].contains(modifier) || add.contains(modifier) {
                                return Err("Duplicate regular expression modifiers");
                            }
                        }
                    }
                }
                if self.eat(':') {
                    self.disjunction()?;
                    if self.eat(')') {
                        return Ok(true);
                    }
                    return Err("Unterminated group");
                }
            }
            self.pos = start;
        }
        Ok(false)
    }

    fn eat_capturing_group(&mut self) -> Result<bool, Raised> {
        if self.eat('(') {
            self.group_specifier()?;
            self.disjunction()?;
            if self.eat(')') {
                self.num_capturing_parens += 1.0;
                return Ok(true);
            }
            return Err("Unterminated group");
        }
        Ok(false)
    }

    fn eat_modifiers(&mut self) -> Vec<i32> {
        let mut modifiers = Vec::new();
        loop {
            let c = self.current();
            if c == END || !is_regular_expression_modifier(c) {
                return modifiers;
            }
            modifiers.push(c);
            self.advance();
        }
    }

    fn eat_extended_atom(&mut self) -> Result<bool, Raised> {
        Ok(self.eat('.')
            || self.eat_reverse_solidus_atom_escape()?
            || self.eat_character_class()?
            || self.eat_uncapturing_group()?
            || self.eat_capturing_group()?
            || self.eat_invalid_braced_quantifier()?
            || self.eat_extended_pattern_character())
    }

    fn eat_invalid_braced_quantifier(&mut self) -> Result<bool, Raised> {
        if self.eat_braced_quantifier(true)? {
            return Err("Nothing to repeat");
        }
        Ok(false)
    }

    fn eat_syntax_character(&mut self) -> bool {
        let c = self.current();
        if is_syntax_character(c) {
            self.last_int_value = f64::from(c);
            self.advance();
            return true;
        }
        false
    }

    fn eat_pattern_characters(&mut self) -> bool {
        let start = self.pos;
        loop {
            let c = self.current();
            if c == END || is_syntax_character(c) {
                break;
            }
            self.advance();
        }
        self.pos != start
    }

    fn eat_extended_pattern_character(&mut self) -> bool {
        let c = self.current();
        if c != END
            && c != ch('$')
            && !(ch('(')..=ch('+')).contains(&c)
            && c != ch('.')
            && c != ch('?')
            && c != ch('[')
            && c != ch('^')
            && c != ch('|')
        {
            self.advance();
            return true;
        }
        false
    }

    fn group_specifier(&mut self) -> Result<(), Raised> {
        if self.eat('?') {
            if !self.eat_group_name()? {
                return Err("Invalid group");
            }
            let name = self.last_string_value.clone();
            if let Some(known) = self.group_names.get(&name) {
                for &other in known {
                    if !self.separated(other, self.branch) {
                        return Err("Duplicate capture group name");
                    }
                }
            }
            self.group_names.entry(name).or_default().push(self.branch);
        }
        Ok(())
    }

    fn eat_group_name(&mut self) -> Result<bool, Raised> {
        self.last_string_value.clear();
        if self.eat('<') {
            if self.eat_identifier_name()? && self.eat('>') {
                return Ok(true);
            }
            return Err("Invalid capture group name");
        }
        Ok(false)
    }

    fn eat_identifier_name(&mut self) -> Result<bool, Raised> {
        self.last_string_value.clear();
        if self.eat_identifier_character(is_identifier_start)? {
            self.push_last_code_point();
            while self.eat_identifier_character(is_identifier_part)? {
                self.push_last_code_point();
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn push_last_code_point(&mut self) {
        let c = char::from_u32(self.last_int_value as u32).unwrap_or(char::REPLACEMENT_CHARACTER);
        self.last_string_value.push(c);
    }

    /// `regexp_eatRegExpIdentifierStart` / `…Part`: one character of a
    /// group name, written out or as a `\u` escape. A name is always
    /// read by code point.
    fn eat_identifier_character(&mut self, accepts: fn(i32) -> bool) -> Result<bool, Raised> {
        let start = self.pos;
        let mut c = self.at(self.pos, true);
        self.pos = self.next_index(self.pos, true);

        if c == ch('\\') && self.eat_unicode_escape_sequence(true)? {
            c = self.last_int_value as i32;
        }
        if accepts(c) {
            self.last_int_value = f64::from(c);
            return Ok(true);
        }
        self.pos = start;
        Ok(false)
    }

    fn eat_atom_escape(&mut self) -> Result<bool, Raised> {
        if self.eat_back_reference()
            || self.eat_character_class_escape()?.read()
            || self.eat_character_escape()?
            || (self.switch_n && self.eat_k_group_name()?)
        {
            return Ok(true);
        }
        if self.switch_u {
            if self.current() == ch('c') {
                return Err("Invalid unicode escape");
            }
            return Err("Invalid escape");
        }
        Ok(false)
    }

    fn eat_back_reference(&mut self) -> bool {
        let start = self.pos;
        if self.eat_decimal_escape() {
            let n = self.last_int_value;
            if self.switch_u {
                if n > self.max_back_reference {
                    self.max_back_reference = n;
                }
                return true;
            }
            if n <= self.num_capturing_parens {
                return true;
            }
            self.pos = start;
        }
        false
    }

    fn eat_k_group_name(&mut self) -> Result<bool, Raised> {
        if self.eat('k') {
            if self.eat_group_name()? {
                self.back_reference_names
                    .push(self.last_string_value.clone());
                return Ok(true);
            }
            return Err("Invalid named reference");
        }
        Ok(false)
    }

    fn eat_character_escape(&mut self) -> Result<bool, Raised> {
        Ok(self.eat_control_escape()
            || self.eat_c_control_letter()
            || self.eat_zero()
            || self.eat_hex_escape_sequence()?
            || self.eat_unicode_escape_sequence(false)?
            || (!self.switch_u && self.eat_legacy_octal_escape_sequence())
            || self.eat_identity_escape())
    }

    fn eat_c_control_letter(&mut self) -> bool {
        let start = self.pos;
        if self.eat('c') {
            if self.eat_control_letter() {
                return true;
            }
            self.pos = start;
        }
        false
    }

    fn eat_zero(&mut self) -> bool {
        if self.current() == ch('0') && !is_decimal_digit(self.lookahead()) {
            self.last_int_value = 0.0;
            self.advance();
            return true;
        }
        false
    }

    fn eat_control_escape(&mut self) -> bool {
        let value = match as_char(self.current()) {
            Some('t') => 0x09,
            Some('n') => 0x0A,
            Some('v') => 0x0B,
            Some('f') => 0x0C,
            Some('r') => 0x0D,
            _ => return false,
        };
        self.last_int_value = f64::from(value);
        self.advance();
        true
    }

    fn eat_control_letter(&mut self) -> bool {
        let c = self.current();
        if is_control_letter(c) {
            self.last_int_value = f64::from(c % 0x20);
            self.advance();
            return true;
        }
        false
    }

    fn eat_unicode_escape_sequence(&mut self, force_u: bool) -> Result<bool, Raised> {
        let start = self.pos;
        let switch_u = force_u || self.switch_u;

        if self.eat('u') {
            if self.eat_fixed_hex_digits(4) {
                let lead = self.last_int_value;
                if switch_u && (f64::from(0xD800)..=f64::from(0xDBFF)).contains(&lead) {
                    let lead_surrogate_end = self.pos;
                    if self.eat('\\') && self.eat('u') && self.eat_fixed_hex_digits(4) {
                        let trail = self.last_int_value;
                        if (f64::from(0xDC00)..=f64::from(0xDFFF)).contains(&trail) {
                            self.last_int_value =
                                (lead - 55296.0) * 1024.0 + (trail - 56320.0) + 65536.0;
                            return Ok(true);
                        }
                    }
                    self.pos = lead_surrogate_end;
                    self.last_int_value = lead;
                }
                return Ok(true);
            }
            if switch_u
                && self.eat('{')
                && self.eat_hex_digits()
                && self.eat('}')
                && is_valid_unicode(self.last_int_value)
            {
                return Ok(true);
            }
            if switch_u {
                return Err("Invalid unicode escape");
            }
            self.pos = start;
        }
        Ok(false)
    }

    fn eat_identity_escape(&mut self) -> bool {
        if self.switch_u {
            if self.eat_syntax_character() {
                return true;
            }
            if self.eat('/') {
                self.last_int_value = f64::from(ch('/'));
                return true;
            }
            return false;
        }
        let c = self.current();
        if c != ch('c') && (!self.switch_n || c != ch('k')) {
            self.last_int_value = f64::from(c);
            self.advance();
            return true;
        }
        false
    }

    fn eat_decimal_escape(&mut self) -> bool {
        self.last_int_value = 0.0;
        let mut c = self.current();
        if (ch('1')..=ch('9')).contains(&c) {
            loop {
                self.last_int_value = 10.0 * self.last_int_value + f64::from(c - ch('0'));
                self.advance();
                c = self.current();
                if !is_decimal_digit(c) {
                    break;
                }
            }
            return true;
        }
        false
    }

    fn eat_character_class_escape(&mut self) -> Result<CharSet, Raised> {
        let c = self.current();
        if is_character_class_escape(c) {
            self.last_int_value = -1.0;
            self.advance();
            return Ok(CharSet::Ok);
        }
        if self.switch_u && (c == ch('P') || c == ch('p')) {
            let negate = c == ch('P');
            self.last_int_value = -1.0;
            self.advance();
            if self.eat('{') {
                let result = self.eat_unicode_property_value_expression()?;
                if result.read() && self.eat('}') {
                    if negate && result == CharSet::String {
                        return Err("Invalid property name");
                    }
                    return Ok(result);
                }
            }
            return Err("Invalid property name");
        }
        Ok(CharSet::None)
    }

    fn eat_unicode_property_value_expression(&mut self) -> Result<CharSet, Raised> {
        let start = self.pos;

        // UnicodePropertyName `=` UnicodePropertyValue
        if self.eat_property_word(is_unicode_property_name_character) && self.eat('=') {
            let name = self.last_string_value.clone();
            if self.eat_property_word(is_unicode_property_value_character) {
                let Some(values) = non_binary_values(&name) else {
                    return Err("Invalid property name");
                };
                if !listed(values, &self.last_string_value) {
                    return Err("Invalid property value");
                }
                return Ok(CharSet::Ok);
            }
        }
        self.pos = start;

        // LoneUnicodePropertyNameOrValue
        if self.eat_property_word(is_unicode_property_value_character) {
            if listed(properties::BINARY, &self.last_string_value) {
                return Ok(CharSet::Ok);
            }
            if self.switch_v && listed(properties::BINARY_OF_STRINGS, &self.last_string_value) {
                return Ok(CharSet::String);
            }
            return Err("Invalid property name");
        }
        Ok(CharSet::None)
    }

    /// A run of characters `accepts` takes, kept in `last_string_value`.
    fn eat_property_word(&mut self, accepts: fn(i32) -> bool) -> bool {
        self.last_string_value.clear();
        loop {
            let c = self.current();
            if !accepts(c) {
                break;
            }
            if let Some(c) = as_char(c) {
                self.last_string_value.push(c);
            }
            self.advance();
        }
        !self.last_string_value.is_empty()
    }

    fn eat_character_class(&mut self) -> Result<bool, Raised> {
        if self.eat('[') {
            let negate = self.eat('^');
            let result = self.class_contents()?;
            if !self.eat(']') {
                return Err("Unterminated character class");
            }
            if negate && result == CharSet::String {
                return Err("Negated character class may contain strings");
            }
            return Ok(true);
        }
        Ok(false)
    }

    fn class_contents(&mut self) -> Result<CharSet, Raised> {
        self.nested(Self::class_contents_inner)
    }

    fn class_contents_inner(&mut self) -> Result<CharSet, Raised> {
        if self.current() == ch(']') {
            return Ok(CharSet::Ok);
        }
        if self.switch_v {
            return self.class_set_expression();
        }
        self.non_empty_class_ranges()?;
        Ok(CharSet::Ok)
    }

    fn non_empty_class_ranges(&mut self) -> Result<(), Raised> {
        while self.eat_class_atom()? {
            let left = self.last_int_value;
            if self.eat('-') && self.eat_class_atom()? {
                let right = self.last_int_value;
                if self.switch_u && (left == -1.0 || right == -1.0) {
                    return Err("Invalid character class");
                }
                if left != -1.0 && right != -1.0 && left > right {
                    return Err("Range out of order in character class");
                }
            }
        }
        Ok(())
    }

    fn eat_class_atom(&mut self) -> Result<bool, Raised> {
        let start = self.pos;

        if self.eat('\\') {
            if self.eat_class_escape()? {
                return Ok(true);
            }
            if self.switch_u {
                let c = self.current();
                if c == ch('c') || is_octal_digit(c) {
                    return Err("Invalid class escape");
                }
                return Err("Invalid escape");
            }
            self.pos = start;
        }

        let c = self.current();
        // acorn's tokenizer ends a literal only outside a class, so the
        // pattern never runs out here; stopping keeps the loop finite.
        if c != ch(']') && c != END {
            self.last_int_value = f64::from(c);
            self.advance();
            return Ok(true);
        }
        Ok(false)
    }

    fn eat_class_escape(&mut self) -> Result<bool, Raised> {
        let start = self.pos;

        if self.eat('b') {
            self.last_int_value = 8.0;
            return Ok(true);
        }
        if self.switch_u && self.eat('-') {
            self.last_int_value = f64::from(ch('-'));
            return Ok(true);
        }
        if !self.switch_u && self.eat('c') {
            if self.eat_class_control_letter() {
                return Ok(true);
            }
            self.pos = start;
        }
        Ok(self.eat_character_class_escape()?.read() || self.eat_character_escape()?)
    }

    fn class_set_expression(&mut self) -> Result<CharSet, Raised> {
        let mut result = CharSet::Ok;
        if self.eat_class_set_range()? {
            // A range; more of the union follows below.
        } else {
            let operand = self.eat_class_set_operand()?;
            if !operand.read() {
                return Err("Invalid character in character class");
            }
            if operand == CharSet::String {
                result = CharSet::String;
            }
            // ClassIntersection
            let start = self.pos;
            while self.eat_chars(['&', '&']) {
                if self.current() != ch('&') {
                    let operand = self.eat_class_set_operand()?;
                    if operand.read() {
                        if operand != CharSet::String {
                            result = CharSet::Ok;
                        }
                        continue;
                    }
                }
                return Err("Invalid character in character class");
            }
            if start != self.pos {
                return Ok(result);
            }
            // ClassSubtraction
            while self.eat_chars(['-', '-']) {
                if self.eat_class_set_operand()?.read() {
                    continue;
                }
                return Err("Invalid character in character class");
            }
            if start != self.pos {
                return Ok(result);
            }
        }
        // ClassUnion
        loop {
            if self.eat_class_set_range()? {
                continue;
            }
            let operand = self.eat_class_set_operand()?;
            if !operand.read() {
                return Ok(result);
            }
            if operand == CharSet::String {
                result = CharSet::String;
            }
        }
    }

    fn eat_class_set_range(&mut self) -> Result<bool, Raised> {
        let start = self.pos;
        if self.eat_class_set_character()? {
            let left = self.last_int_value;
            if self.eat('-') && self.eat_class_set_character()? {
                let right = self.last_int_value;
                if left != -1.0 && right != -1.0 && left > right {
                    return Err("Range out of order in character class");
                }
                return Ok(true);
            }
            self.pos = start;
        }
        Ok(false)
    }

    fn eat_class_set_operand(&mut self) -> Result<CharSet, Raised> {
        if self.eat_class_set_character()? {
            return Ok(CharSet::Ok);
        }
        let disjunction = self.eat_class_string_disjunction()?;
        if disjunction.read() {
            return Ok(disjunction);
        }
        self.eat_nested_class()
    }

    fn eat_nested_class(&mut self) -> Result<CharSet, Raised> {
        let start = self.pos;
        if self.eat('[') {
            let negate = self.eat('^');
            let result = self.class_contents()?;
            if self.eat(']') {
                if negate && result == CharSet::String {
                    return Err("Negated character class may contain strings");
                }
                return Ok(result);
            }
            self.pos = start;
        }
        if self.eat('\\') {
            let result = self.eat_character_class_escape()?;
            if result.read() {
                return Ok(result);
            }
            self.pos = start;
        }
        Ok(CharSet::None)
    }

    fn eat_class_string_disjunction(&mut self) -> Result<CharSet, Raised> {
        let start = self.pos;
        if self.eat_chars(['\\', 'q']) {
            if self.eat('{') {
                let result = self.class_string_disjunction_contents()?;
                if self.eat('}') {
                    return Ok(result);
                }
            } else {
                return Err("Invalid escape");
            }
            self.pos = start;
        }
        Ok(CharSet::None)
    }

    fn class_string_disjunction_contents(&mut self) -> Result<CharSet, Raised> {
        let mut result = self.class_string()?;
        while self.eat('|') {
            if self.class_string()? == CharSet::String {
                result = CharSet::String;
            }
        }
        Ok(result)
    }

    fn class_string(&mut self) -> Result<CharSet, Raised> {
        let mut count = 0;
        while self.eat_class_set_character()? {
            count += 1;
        }
        Ok(if count == 1 {
            CharSet::Ok
        } else {
            CharSet::String
        })
    }

    fn eat_class_set_character(&mut self) -> Result<bool, Raised> {
        let start = self.pos;
        if self.eat('\\') {
            if self.eat_character_escape()? || self.eat_class_set_reserved_punctuator() {
                return Ok(true);
            }
            if self.eat('b') {
                self.last_int_value = 8.0;
                return Ok(true);
            }
            self.pos = start;
            return Ok(false);
        }
        let c = self.current();
        if c < 0
            || (c == self.lookahead() && is_class_set_reserved_double_punctuator_character(c))
            || is_class_set_syntax_character(c)
        {
            return Ok(false);
        }
        self.advance();
        self.last_int_value = f64::from(c);
        Ok(true)
    }

    fn eat_class_set_reserved_punctuator(&mut self) -> bool {
        let c = self.current();
        if is_class_set_reserved_punctuator(c) {
            self.last_int_value = f64::from(c);
            self.advance();
            return true;
        }
        false
    }

    fn eat_class_control_letter(&mut self) -> bool {
        let c = self.current();
        if is_decimal_digit(c) || c == ch('_') {
            self.last_int_value = f64::from(c % 0x20);
            self.advance();
            return true;
        }
        false
    }

    fn eat_hex_escape_sequence(&mut self) -> Result<bool, Raised> {
        let start = self.pos;
        if self.eat('x') {
            if self.eat_fixed_hex_digits(2) {
                return Ok(true);
            }
            if self.switch_u {
                return Err("Invalid escape");
            }
            self.pos = start;
        }
        Ok(false)
    }

    fn eat_decimal_digits(&mut self) -> bool {
        let start = self.pos;
        self.last_int_value = 0.0;
        loop {
            let c = self.current();
            if !is_decimal_digit(c) {
                break;
            }
            self.last_int_value = 10.0 * self.last_int_value + f64::from(c - ch('0'));
            self.advance();
        }
        self.pos != start
    }

    fn eat_hex_digits(&mut self) -> bool {
        let start = self.pos;
        self.last_int_value = 0.0;
        loop {
            let c = self.current();
            if !is_hex_digit(c) {
                break;
            }
            self.last_int_value = 16.0 * self.last_int_value + f64::from(hex_to_int(c));
            self.advance();
        }
        self.pos != start
    }

    fn eat_legacy_octal_escape_sequence(&mut self) -> bool {
        if self.eat_octal_digit() {
            let n1 = self.last_int_value;
            if self.eat_octal_digit() {
                let n2 = self.last_int_value;
                if n1 <= 3.0 && self.eat_octal_digit() {
                    let n3 = self.last_int_value;
                    self.last_int_value = n1 * 64.0 + n2 * 8.0 + n3;
                } else {
                    self.last_int_value = n1 * 8.0 + n2;
                }
            } else {
                self.last_int_value = n1;
            }
            return true;
        }
        false
    }

    fn eat_octal_digit(&mut self) -> bool {
        let c = self.current();
        if is_octal_digit(c) {
            self.last_int_value = f64::from(c - ch('0'));
            self.advance();
            return true;
        }
        self.last_int_value = 0.0;
        false
    }

    fn eat_fixed_hex_digits(&mut self, length: usize) -> bool {
        let start = self.pos;
        self.last_int_value = 0.0;
        for _ in 0..length {
            let c = self.current();
            if !is_hex_digit(c) {
                self.pos = start;
                return false;
            }
            self.last_int_value = 16.0 * self.last_int_value + f64::from(hex_to_int(c));
            self.advance();
        }
        true
    }
}
