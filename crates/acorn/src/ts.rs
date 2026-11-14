//! acorn-typescript's own machinery: reading ahead, trying a reading and
//! taking it back, modifiers, decorators, import attributes.
//!
//! acorn-typescript extends acorn's parser class and overrides its
//! methods. Here the parser is one type with a `ts` switch: a method
//! acorn-typescript overrides asks the switch and goes to its `ts_`
//! counterpart, which calls back into the plain method where the
//! override calls `super`.

use crate::identifier::is_identifier_char;
use crate::state::{
    Bind, Names, Node, PResult, Parser, Raised, SCOPE_TOP, Stop, Ty, add_name, is_new_line,
};
use crate::token::{Ctx, Tok};

/// A token read ahead of the parser's position.
pub(crate) struct Ahead {
    pub ty: Tok,
    pub value: String,
    pub contains_esc: bool,
}

/// How a tried reading ended: with a node, with an error, or given up.
pub(crate) struct Tried<T> {
    pub node: Option<T>,
    pub error: Option<Raised>,
    pub aborted: bool,
}

impl<T> Tried<T> {
    pub(crate) fn thrown(&self) -> bool {
        self.error.is_some()
    }
}

/// The lists acorn-typescript walks with one loop, told apart by what
/// ends them.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ListKind {
    EnumMembers,
    TypeMembers,
    HeritageClauseElement,
    TupleElementTypes,
    TypeParametersOrArguments,
}

/// What `tsParseModifiers` is asked to read.
pub(crate) struct ModifierRules<'a> {
    pub allowed: &'a [&'a str],
    pub disallowed: &'a [&'a str],
    pub stop_on_start_of_class_static_block: bool,
    /// The error for a modifier from `disallowed`.
    pub error: &'static str,
}

// acorn-typescript raises the error for a disallowed modifier with the
// function that would build the message, not with the message. What
// reaches the user is that function's source text, so that text is
// the message here.
pub(crate) const INVALID_MODIFIER_ON_TYPE_MEMBER: &str =
    "({ modifier }) => `'${modifier}' modifier cannot appear on a type member.`";
pub(crate) const INVALID_MODIFIER_ON_TYPE_PARAMETER: &str =
    "({ modifier }) => `'${modifier}' modifier cannot appear on a type parameter.`";
pub(crate) const INVALID_MODIFIER_ON_TYPE_PARAMETER_POSITIONS: &str = "({ modifier }) => `'${modifier}' modifier can only appear on a type parameter of a class, interface or type alias.`";

fn is_access_modifier(modifier: &str) -> bool {
    matches!(modifier, "private" | "public" | "protected")
}

impl Parser {
    // --- tokens ---

    pub(crate) fn ts_is_contextual(&self, ty: Tok) -> bool {
        self.ty == ty && !self.contains_esc
    }

    pub(crate) fn ts_match_left_relational(&self) -> bool {
        self.ty == Tok::Relational && self.value == "<"
    }

    pub(crate) fn ts_match_right_relational(&self) -> bool {
        self.ty == Tok::Relational && self.value == ">"
    }

    pub(crate) fn has_preceding_line_break(&self) -> bool {
        self.line_break_between(self.last_tok_end, self.start)
    }

    pub(crate) fn is_line_terminator(&mut self) -> PResult<bool> {
        Ok(self.eat(Tok::Semi)? || self.can_insert_semicolon())
    }

    /// Whether only white space and comments that end on this line
    /// separate the current token from the end of the line.
    pub(crate) fn has_following_line_break(&self) -> bool {
        let len = self.input.len();
        let mut i = self.end;
        loop {
            let c = self.code_at(i);
            let inline_space = matches!(
                c,
                9 | 11 | 12 | 32 | 0xa0 | 0x1680 | 0x2000
                    ..=0x200a | 0x202f | 0x205f | 0x3000 | 0xfeff
            );
            if inline_space {
                i += 1;
            } else if c == 47 && self.code_at(i + 1) == 47 {
                i += 2;
                while i < len && !is_new_line(self.code_at(i)) {
                    i += 1;
                }
            } else if c == 47 && self.code_at(i + 1) == 42 {
                // A comment that closes before the line ends.
                let mut close = i + 2;
                let mut found = false;
                while close + 1 < len && !is_new_line(self.code_at(close)) {
                    if self.input[close] == 42 && self.input[close + 1] == 47 {
                        found = true;
                        break;
                    }
                    close += 1;
                }
                if !found {
                    // One that runs past the line counts as its break.
                    return true;
                }
                i = close + 2;
            } else {
                break;
            }
        }
        i >= len || is_new_line(self.code_at(i))
    }

    pub(crate) fn next_token_start_since(&self, pos: usize) -> usize {
        pos + self.skip_white_space_len(pos)
    }

    pub(crate) fn next_token_start(&self) -> usize {
        self.next_token_start_since(self.pos)
    }

    pub(crate) fn lookahead_char_code(&self) -> i32 {
        self.code_at(self.next_token_start())
    }

    /// Whether the unread text at `start` is the word `name`.
    pub(crate) fn is_unparsed_contextual(&self, start: usize, name: &str) -> bool {
        let end = start + name.len();
        if !self.slice_is(start, end, name) || end > self.input.len() {
            return false;
        }
        let next = self.code_at(end);
        // The second test is for the first half of a surrogate pair.
        !(is_identifier_char(next) || (next >= 0 && (next & 0xfc00) == 0xd800))
    }

    pub(crate) fn is_lookahead_contextual(&self, name: &str) -> bool {
        self.is_unparsed_contextual(self.next_token_start(), name)
    }

    /// The token `count` tokens ahead, read without moving on.
    pub(crate) fn lookahead(&mut self, count: usize) -> PResult<Ahead> {
        let saved = (
            self.pos,
            std::mem::take(&mut self.value),
            self.value_lone_surrogate,
            self.ty,
            self.expr_allowed,
            self.start,
            self.end,
            self.last_tok_start,
            self.last_tok_end,
            self.contains_esc,
        );
        let old_context = std::mem::take(&mut self.context);
        self.context = vec![old_context.last().copied().unwrap_or(Ctx::BStat)];
        let old_is_lookahead = self.is_lookahead;
        self.is_lookahead = true;

        let mut result = Ok(());
        for _ in 0..count {
            result = self.next_token();
            if result.is_err() {
                break;
            }
        }
        let ahead = Ahead {
            ty: self.ty,
            value: self.value.clone(),
            contains_esc: self.contains_esc,
        };

        (
            self.pos,
            self.value,
            self.value_lone_surrogate,
            self.ty,
            self.expr_allowed,
            self.start,
            self.end,
            self.last_tok_start,
            self.last_tok_end,
            self.contains_esc,
        ) = saved;
        self.context = old_context;
        self.is_lookahead = old_is_lookahead;
        result.map(|()| ahead)
    }

    /// Reads a `>` the type parser took alone as the operator it starts.
    pub(crate) fn re_scan_lt_gt(&mut self) {
        if self.ty == Tok::Relational {
            self.pos -= 1;
            let code = self.full_char_code_at(self.pos);
            self.read_token_lt_gt(code);
        }
    }

    /// Reads a `<<` as the `<` that opens type arguments.
    pub(crate) fn re_scan_lt(&mut self) -> Tok {
        if self.ty == Tok::BitShift {
            self.pos -= 2;
            self.finish_op(Tok::Relational, 1);
            return Tok::Relational;
        }
        self.ty
    }

    // --- trying a reading ---

    /// Runs `f`. Where it throws or gives up (`Ok(None)`), the parser is
    /// put back as it was.
    pub(crate) fn try_parse<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> PResult<Option<T>>,
    ) -> PResult<Tried<T>> {
        let snapshot = self.clone();
        Ok(match f(self) {
            Ok(Some(node)) => Tried {
                node: Some(node),
                error: None,
                aborted: false,
            },
            Ok(None) => {
                *self = snapshot;
                Tried {
                    node: None,
                    error: None,
                    aborted: true,
                }
            }
            Err(stop) => {
                *self = snapshot;
                let error = match stop {
                    Stop::Error(raised) => raised,
                    Stop::Crash(message) => return Err(Stop::Crash(message)),
                    Stop::StackExhausted(raised) => return Err(Stop::StackExhausted(raised)),
                    Stop::InvalidTemplateEscape => Raised {
                        pos: self.start as u32,
                        message: "Unexpected token".to_string(),
                        column_of: false,
                        no_position: false,
                    },
                };
                Tried {
                    node: None,
                    error: Some(error),
                    aborted: false,
                }
            }
        })
    }

    /// `tsTryParse`: keeps what `f` read when it returns something,
    /// takes it back when it returns nothing or throws.
    pub(crate) fn ts_try_parse<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> PResult<Option<T>>,
    ) -> PResult<Option<T>> {
        let snapshot = self.clone();
        match f(self) {
            Ok(Some(value)) => Ok(Some(value)),
            Ok(None) => {
                *self = snapshot;
                Ok(None)
            }
            Err(stop) => {
                *self = snapshot;
                Err(stop)
            }
        }
    }

    /// `tsLookAhead`: runs `f` and takes back whatever it read.
    pub(crate) fn ts_look_ahead<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<T> {
        let snapshot = self.clone();
        let result = f(self);
        *self = snapshot;
        result
    }

    /// `tsTryParseAndCatch`: what `f` read, or nothing where it threw
    /// or read nothing.
    pub(crate) fn ts_try_parse_and_catch<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> PResult<Option<T>>,
    ) -> PResult<Option<T>> {
        Ok(self.try_parse(f)?.node)
    }

    pub(crate) fn rethrow<T>(&self, error: Raised) -> PResult<T> {
        Err(Stop::Error(error))
    }

    // --- contexts ---

    pub(crate) fn ts_in_type<T>(&mut self, f: impl FnOnce(&mut Self) -> PResult<T>) -> PResult<T> {
        let old = self.in_type;
        self.in_type = true;
        let result = f(self);
        self.in_type = old;
        result
    }

    pub(crate) fn ts_in_no_context<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<T> {
        let old = std::mem::take(&mut self.context);
        self.context = vec![old.first().copied().unwrap_or(Ctx::BStat)];
        let result = f(self);
        self.context = old;
        result
    }

    pub(crate) fn ts_in_ambient_context<T>(
        &mut self,
        f: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<T> {
        let old = self.is_ambient_context;
        self.is_ambient_context = true;
        let result = f(self);
        self.is_ambient_context = old;
        result
    }

    pub(crate) fn ts_in_disallow_conditional_types_context<T>(
        &mut self,
        disallow: bool,
        f: impl FnOnce(&mut Self) -> PResult<T>,
    ) -> PResult<T> {
        let old = self.in_disallow_conditional_types_context;
        self.in_disallow_conditional_types_context = disallow;
        let result = f(self);
        self.in_disallow_conditional_types_context = old;
        result
    }

    // --- errors ---

    /// `raise` for "Comma is not permitted after the rest element". In
    /// an ambient context acorn-typescript lets a comma before the `)`
    /// pass and steps over it.
    pub(crate) fn raise_comma_after_rest(&mut self, pos: usize) -> PResult<()> {
        if self.ts
            && self.is_ambient_context
            && self.ty == Tok::Comma
            && self.lookahead_char_code() == 41
        {
            return self.next();
        }
        self.raise(pos, "Comma is not permitted after the rest element")
    }

    // --- scopes ---

    fn maybe_export_defined(&mut self, name: &str) {
        if self.current_scope().flags & SCOPE_TOP != 0 {
            self.forget_undefined_export(name);
        }
    }

    /// acorn-typescript's `declareName`: its own kinds of binding, and
    /// acorn's for the rest.
    pub(crate) fn ts_declare_name(&mut self, name: &str, binding: Bind, pos: u32) -> PResult<()> {
        let last = self.scope_stack.len() - 1;
        match binding {
            Bind::TsImport => {
                if self.ts_has_import(name, true) {
                    return self.raise(
                        pos as usize,
                        format!("Identifier '{name}' has already been declared."),
                    );
                }
                if let Some(imports) = self.imports_stack.last_mut() {
                    add_name(imports, name);
                }
                Ok(())
            }
            Bind::TsExportOnly => {
                self.maybe_export_defined(name);
                add_name(&mut self.scope_stack[last].export_only_bindings, name);
                Ok(())
            }
            Bind::TsType | Bind::TsInterface => {
                if binding == Bind::TsType && self.scope_stack[last].types.iter().any(|n| n == name)
                {
                    return self.raise(
                        pos as usize,
                        format!("type '{name}' has already been declared."),
                    );
                }
                add_name(&mut self.scope_stack[last].types, name);
                Ok(())
            }
            Bind::TsEnum | Bind::TsConstEnum => {
                if self.scope_stack[last].enums.iter().any(|n| n == name) {
                    return Ok(());
                }
                self.declare_name_base(name, Bind::Lexical, pos)?;
                add_name(&mut self.scope_stack[last].enums, name);
                Ok(())
            }
            _ => self.declare_name_base(name, binding, pos),
        }
    }

    /// Whether `name` is imported where the parser stands; unless
    /// `allow_shadow`, in a namespace around it too.
    pub(crate) fn ts_has_import(&self, name: &str, allow_shadow: bool) -> bool {
        let has = |names: &Names| names.iter().any(|n| n == name);
        match self.imports_stack.split_last() {
            Some((last, outer)) => has(last) || (!allow_shadow && outer.iter().any(has)),
            None => false,
        }
    }

    /// Whether `name` is a type or something only an `export` sees, in
    /// any scope: then an `export { name }` needs no declaration.
    pub(crate) fn ts_exportable_without_binding(&self, name: &str) -> bool {
        self.scope_stack.iter().any(|scope| {
            scope.types.iter().any(|n| n == name)
                || scope.export_only_bindings.iter().any(|n| n == name)
        })
    }

    // --- modifiers ---

    fn ts_token_can_follow_modifier(&self) -> bool {
        (matches!(
            self.ty,
            Tok::BracketL | Tok::BraceL | Tok::Star | Tok::Ellipsis | Tok::PrivateId
        ) || self.ty.is_literal_property_name())
            && !self.has_preceding_line_break()
    }

    pub(crate) fn ts_can_follow_modifier(&self) -> bool {
        self.ts_token_can_follow_modifier()
    }

    pub(crate) fn ts_is_start_of_static_blocks(&self) -> bool {
        self.is_contextual("static") && self.lookahead_char_code() == 123
    }

    /// The modifier the parser stands on, if it is one of `allowed` and
    /// what follows can follow a modifier. Steps over it.
    pub(crate) fn ts_parse_modifier(
        &mut self,
        allowed: &[&str],
        stop_on_start_of_class_static_block: bool,
    ) -> PResult<Option<String>> {
        let modifier = self.value.clone();
        if allowed.contains(&modifier.as_str()) && !self.contains_esc {
            if stop_on_start_of_class_static_block && self.ts_is_start_of_static_blocks() {
                return Ok(None);
            }
            let follows = self.ts_try_parse(|p| {
                p.next_liberal(true)?;
                Ok(p.ts_token_can_follow_modifier().then_some(()))
            })?;
            if follows.is_some() {
                return Ok(Some(modifier));
            }
        }
        Ok(None)
    }

    /// `tsParseModifiers`: reads modifiers onto `modified`, checking
    /// their order and combinations. Returns the ones it read.
    pub(crate) fn ts_parse_modifiers(
        &mut self,
        modified: &mut Node,
        rules: &ModifierRules<'_>,
    ) -> PResult<Vec<String>> {
        let mut read = Vec::new();
        let all: Vec<&str> = rules
            .allowed
            .iter()
            .chain(rules.disallowed.iter())
            .copied()
            .collect();
        loop {
            let start = self.start;
            let Some(modifier) =
                self.ts_parse_modifier(&all, rules.stop_on_start_of_class_static_block)?
            else {
                break;
            };
            let modifier = modifier.as_str();
            // acorn-typescript raises these two at the modifier's column
            // number, taken for an offset.
            let enforce_order = |parser: &Parser, modified: &Node, before: &str, after: &str| {
                if modifier == before && modified.has_modifier(after) {
                    return parser.raise_at_column_of::<()>(
                        start,
                        format!("'{before}' modifier must precede '{after}' modifier."),
                    );
                }
                Ok(())
            };
            let incompatible = |parser: &Parser, modified: &Node, mod1: &str, mod2: &str| {
                if (modified.has_modifier(mod1) && modifier == mod2)
                    || (modified.has_modifier(mod2) && modifier == mod1)
                {
                    return parser.raise_at_column_of::<()>(
                        start,
                        format!("'{mod1}' modifier cannot be used with '{mod2}' modifier."),
                    );
                }
                Ok(())
            };
            let duplicate = |parser: &Parser| {
                parser.raise::<()>(parser.start, format!("Duplicate modifier: '{modifier}'."))
            };

            if is_access_modifier(modifier) {
                if modified.accessibility.is_some() {
                    return self.raise(self.start, "Accessibility modifier already seen.");
                }
                enforce_order(self, modified, modifier, "override")?;
                enforce_order(self, modified, modifier, "static")?;
                enforce_order(self, modified, modifier, "readonly")?;
                enforce_order(self, modified, modifier, "accessor")?;
                modified.accessibility = Some(modifier.to_string());
                read.push("accessibility".to_string());
            } else if modifier == "in" || modifier == "out" {
                if modified.has_modifier(modifier) {
                    duplicate(self)?;
                }
                enforce_order(self, modified, "in", "out")?;
                modified.modifiers.push(modifier.to_string());
                read.push(modifier.to_string());
            } else if modifier == "accessor" {
                if modified.has_modifier(modifier) {
                    duplicate(self)?;
                }
                incompatible(self, modified, "accessor", "readonly")?;
                modified.modifiers.push(modifier.to_string());
                read.push(modifier.to_string());
            } else if modifier == "const" {
                if modified.has_modifier(modifier) {
                    duplicate(self)?;
                }
                modified.modifiers.push(modifier.to_string());
                read.push(modifier.to_string());
            } else {
                if modified.has_modifier(modifier) {
                    duplicate(self)?;
                }
                enforce_order(self, modified, "static", "readonly")?;
                enforce_order(self, modified, "static", "override")?;
                enforce_order(self, modified, "override", "readonly")?;
                enforce_order(self, modified, "abstract", "override")?;
                incompatible(self, modified, "declare", "override")?;
                incompatible(self, modified, "static", "abstract")?;
                modified.modifiers.push(modifier.to_string());
                read.push(modifier.to_string());
            }

            if rules.disallowed.contains(&modifier) {
                return self.raise(self.start, rules.error);
            }
        }
        Ok(read)
    }

    // --- decorators ---

    /// `@name`, `@name.member`, `@(expression)`, each with optional
    /// arguments.
    pub(crate) fn parse_decorator(&mut self) -> PResult<()> {
        self.next()?;
        self.decorator_stack.push((0, 0));

        let start_pos = self.start;
        let mut expr = if self.ty == Tok::ParenL {
            let paren_start = self.start;
            self.next()?;
            let inner = self.parse_expression(crate::state::ForInit::No, None)?;
            self.expect(Tok::ParenR)?;
            if self.preserve_parens {
                let mut parenthesized = self.start_node_at(paren_start);
                parenthesized.a = Some(Box::new(inner));
                self.finish_node(parenthesized, Ty::ParenthesizedExpression)
            } else {
                inner
            }
        } else {
            self.parse_ident(false)?
        };
        // Members, private ones too, and TypeScript's `!` anywhere along
        // the chain: `@x.#y`, `@x!`, `@x!.y`.
        loop {
            if self.eat(Tok::Dot)? {
                let mut member = self.start_node_at(start_pos);
                let property = if self.ty == Tok::PrivateId {
                    self.parse_private_ident()?
                } else {
                    self.parse_ident(true)?
                };
                member.a = Some(Box::new(expr));
                member.b = Some(Box::new(property));
                expr = self.finish_node(member, Ty::MemberExpression);
            } else if !self.has_preceding_line_break()
                && self.value == "!"
                && self.ty == Tok::Prefix
            {
                self.expr_allowed = false;
                self.next()?;
                let mut asserted = self.start_node_at(start_pos);
                asserted.a = Some(Box::new(expr));
                expr = self.finish_node(asserted, Ty::TsNonNullExpression);
            } else {
                break;
            }
        }

        // parseMaybeDecoratorArguments
        let has_type_arguments = if self.ts_match_left_relational() || self.ty == Tok::BitShift {
            self.ts_parse_type_arguments_in_expression()?
        } else {
            false
        };
        if self.eat(Tok::ParenL)? {
            let call = self.start_node_at(expr.start as usize);
            self.parse_expr_list(Tok::ParenR, false, false, None)?;
            expr = self.finish_node(call, Ty::Other);
        } else if has_type_arguments {
            let instantiation = self.start_node_at(expr.start as usize);
            expr = self.finish_node(instantiation, Ty::TsInstantiationExpression);
        }
        let _ = expr;
        self.decorator_stack.pop();
        Ok(())
    }

    fn is_abstract_class(&mut self) -> PResult<bool> {
        Ok(self.ts_is_contextual(Tok::TsAbstract) && self.lookahead(1)?.ty == Tok::KwClass)
    }

    pub(crate) fn ts_is_abstract_class(&mut self) -> PResult<bool> {
        self.is_abstract_class()
    }

    fn is_declare_class(&self) -> bool {
        if !self.ts_is_contextual(Tok::TsDeclare) {
            return false;
        }
        let after_declare = self.next_token_start();
        if self.is_unparsed_contextual(after_declare, "class") {
            return true;
        }
        if !self.is_unparsed_contextual(after_declare, "abstract") {
            return false;
        }
        let after_abstract = self.next_token_start_since(after_declare + "abstract".len());
        self.is_unparsed_contextual(after_abstract, "class")
    }

    /// The decorators before a statement or an expression.
    pub(crate) fn parse_decorators(&mut self, allow_export: bool) -> PResult<()> {
        while self.ty == Tok::At {
            let start = self.start as u32;
            self.parse_decorator()?;
            if let Some((count, first)) = self.decorator_stack.last_mut() {
                if *count == 0 {
                    *first = start;
                }
                *count += 1;
            }
        }
        if self.ty == Tok::KwExport {
            if !allow_export {
                return self.unexpected();
            }
        } else if !(self.ty == Tok::KwClass || self.is_abstract_class()? || self.is_declare_class())
        {
            return self.raise(
                self.start,
                "Leading decorators must be attached to a class declaration.",
            );
        }
        Ok(())
    }

    /// A class takes the decorators waiting for it, and starts where
    /// the first of them does.
    pub(crate) fn take_decorators(&mut self, node: &mut Node) {
        if let Some((count, first)) = self.decorator_stack.last_mut()
            && *count > 0
        {
            node.start = *first;
            *count = 0;
        }
    }

    // --- import attributes ---

    /// acorn-typescript's `with { … }` / `assert { … }` after an import
    /// or re-export. It steps over one token too many after each key,
    /// which is how a well-formed clause ends up refused.
    pub(crate) fn parse_maybe_import_attributes(&mut self) -> PResult<()> {
        if self.ty != Tok::KwWith && self.ty != Tok::TsAssert {
            return Ok(());
        }
        self.next()?;
        self.expect(Tok::BraceL)?;
        let mut names: Vec<String> = Vec::new();
        loop {
            if self.ty == Tok::BraceR {
                break;
            }
            // A key is a name or a string: `type` and `"type"` are one.
            let name = if self.ty == Tok::String {
                self.parse_literal()?.string.take().unwrap_or_default()
            } else {
                std::mem::take(&mut self.parse_ident(true)?.name)
            };
            self.next()?;
            if names.contains(&name) {
                return self.raise(self.pos, "Duplicated key in attributes");
            }
            names.push(name);
            if self.ty != Tok::String {
                return self.raise(self.pos, "Only string is supported as an attribute value");
            }
            self.parse_literal()?;
            if !self.eat(Tok::Comma)? {
                break;
            }
        }
        self.expect(Tok::BraceR)
    }

    // --- lists ---

    fn ts_is_list_terminator(&self, kind: ListKind) -> bool {
        match kind {
            ListKind::EnumMembers | ListKind::TypeMembers => self.ty == Tok::BraceR,
            ListKind::HeritageClauseElement => self.ty == Tok::BraceL,
            ListKind::TupleElementTypes => self.ty == Tok::BracketR,
            ListKind::TypeParametersOrArguments => self.ts_match_right_relational(),
        }
    }

    /// `tsParseDelimitedList`: elements separated by commas up to the
    /// list's terminator. Also returns where a trailing comma stood.
    pub(crate) fn ts_parse_delimited_list<T>(
        &mut self,
        kind: ListKind,
        mut parse_element: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<(Vec<T>, Option<usize>)> {
        let mut result = Vec::new();
        let mut trailing_comma = None;
        loop {
            if self.ts_is_list_terminator(kind) {
                break;
            }
            trailing_comma = None;
            result.push(parse_element(self)?);
            if self.eat(Tok::Comma)? {
                trailing_comma = Some(self.last_tok_start);
                continue;
            }
            if self.ts_is_list_terminator(kind) {
                break;
            }
            // Neither a comma nor the end.
            self.expect(Tok::Comma)?;
            return self.unexpected();
        }
        Ok((result, trailing_comma))
    }

    /// `tsParseList`: elements one after another up to the terminator.
    pub(crate) fn ts_parse_list<T>(
        &mut self,
        kind: ListKind,
        mut parse_element: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Vec<T>> {
        let mut result = Vec::new();
        while !self.ts_is_list_terminator(kind) {
            result.push(parse_element(self)?);
        }
        Ok(result)
    }
}
