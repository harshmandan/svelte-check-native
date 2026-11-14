//! The parser's state and the small helpers every part of it uses:
//! acorn's `state.js`, `scope.js`, `scopeflags.js`, `node.js`,
//! `parseutil.js` and `location.js`.

use crate::identifier::is_identifier_char;
use crate::token::{Ctx, Tok};

/// The error acorn throws: where (in UTF-16 units of the input) and
/// its message without the `(line:column)` suffix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Raised {
    pub pos: u32,
    pub message: String,
}

/// Why parsing stopped.
#[derive(Debug)]
pub(crate) enum Stop {
    Error(Raised),
    /// An escape a template may not hold. Thrown inside the tokenizer
    /// and caught by it: the template is then read as an invalid one.
    InvalidTemplateEscape,
}

pub(crate) type PResult<T> = Result<T, Stop>;

pub(crate) const SCOPE_TOP: u32 = 1;
pub(crate) const SCOPE_FUNCTION: u32 = 2;
pub(crate) const SCOPE_ASYNC: u32 = 4;
pub(crate) const SCOPE_GENERATOR: u32 = 8;
pub(crate) const SCOPE_ARROW: u32 = 16;
pub(crate) const SCOPE_SIMPLE_CATCH: u32 = 32;
pub(crate) const SCOPE_SUPER: u32 = 64;
pub(crate) const SCOPE_DIRECT_SUPER: u32 = 128;
pub(crate) const SCOPE_CLASS_STATIC_BLOCK: u32 = 256;
pub(crate) const SCOPE_CLASS_FIELD_INIT: u32 = 512;
pub(crate) const SCOPE_SWITCH: u32 = 1024;
pub(crate) const SCOPE_VAR: u32 = SCOPE_TOP | SCOPE_FUNCTION | SCOPE_CLASS_STATIC_BLOCK;

pub(crate) fn function_flags(is_async: bool, generator: bool) -> u32 {
    SCOPE_FUNCTION
        | if is_async { SCOPE_ASYNC } else { 0 }
        | if generator { SCOPE_GENERATOR } else { 0 }
}

/// How a name is bound.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Bind {
    /// Not a binding.
    None,
    Var,
    Lexical,
    Function,
    SimpleCatch,
    /// A function's own name, bound inside the function.
    Outside,
}

pub(crate) struct Scope {
    pub flags: u32,
    pub var: Vec<String>,
    pub lexical: Vec<String>,
    pub functions: Vec<String>,
}

/// The node types the parser tells apart. Everything it only builds
/// and never looks at again is `Other`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ty {
    Identifier,
    PrivateIdentifier,
    Literal,
    Super,
    MemberExpression,
    ChainExpression,
    ParenthesizedExpression,
    ArrowFunctionExpression,
    FunctionExpression,
    FunctionDeclaration,
    ClassDeclaration,
    ObjectExpression,
    ObjectPattern,
    ArrayExpression,
    ArrayPattern,
    Property,
    SpreadElement,
    RestElement,
    AssignmentExpression,
    AssignmentPattern,
    VariableDeclaration,
    VariableDeclarator,
    MethodDefinition,
    PropertyDefinition,
    StaticBlock,
    ExportSpecifier,
    Other,
}

/// A syntax tree node, with only the fields the parser reads back.
/// Which fields a type fills:
///
/// - `Identifier`, `PrivateIdentifier`: `name`
/// - `Literal`: `string` (the value of a string literal)
/// - `MemberExpression`: `a` object, `b` property
/// - `ChainExpression`, `ParenthesizedExpression`: `a` expression
/// - `Property`: `a` key, `b` value, `name` kind, `computed`, `method`,
///   `shorthand`
/// - `SpreadElement`, `RestElement`: `a` argument
/// - `AssignmentExpression`, `AssignmentPattern`: `a` left, `b` right,
///   `name` operator
/// - arrays, objects, their patterns: `list` (a hole is `None`)
/// - `VariableDeclaration`: `list` declarators, `name` kind
/// - `VariableDeclarator`: `a` id, `b` init
/// - functions: `a` id, `list` parameters
/// - `ClassDeclaration`: `a` id
/// - `MethodDefinition`, `PropertyDefinition`: `a` key, `b` value,
///   `name` kind, `computed`, `is_static`
/// - `ExportSpecifier`: `a` local, `b` exported
#[derive(Debug, Clone)]
pub(crate) struct Node {
    pub ty: Ty,
    pub start: u32,
    pub end: u32,
    pub name: String,
    pub string: Option<String>,
    pub computed: bool,
    pub method: bool,
    pub shorthand: bool,
    pub is_static: bool,
    pub optional: bool,
    pub a: Option<Box<Node>>,
    pub b: Option<Box<Node>>,
    pub list: Vec<Option<Node>>,
}

impl Node {
    pub(crate) fn at(start: u32) -> Node {
        Node {
            ty: Ty::Other,
            start,
            end: start,
            name: String::new(),
            string: None,
            computed: false,
            method: false,
            shorthand: false,
            is_static: false,
            optional: false,
            a: None,
            b: None,
            list: Vec::new(),
        }
    }

    pub(crate) fn is(&self, ty: Ty) -> bool {
        self.ty == ty
    }

    /// The node in `a`, which the node's type always fills.
    pub(crate) fn first(&self) -> &Node {
        self.a.as_deref().unwrap_or(self)
    }

    /// The node in `b`, which the node's type always fills.
    pub(crate) fn second(&self) -> &Node {
        self.b.as_deref().unwrap_or(self)
    }
}

/// Positions of constructs that are errors in an expression but fine
/// in a destructuring pattern, or the other way round. Which it is
/// only shows once the parser sees what follows. `-1` is unset.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DestructuringErrors {
    pub shorthand_assign: i64,
    pub trailing_comma: i64,
    pub parenthesized_assign: i64,
    pub parenthesized_bind: i64,
    pub double_proto: i64,
}

impl DestructuringErrors {
    pub(crate) fn new() -> Self {
        Self {
            shorthand_assign: -1,
            trailing_comma: -1,
            parenthesized_assign: -1,
            parenthesized_bind: -1,
            double_proto: -1,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LabelKind {
    Loop,
    Switch,
}

pub(crate) struct Label {
    pub kind: Option<LabelKind>,
    pub name: Option<String>,
    pub statement_start: Option<u32>,
}

/// The private names of one class body.
pub(crate) struct PrivateNames {
    /// Declared names, with what declared them (`"true"`, `"iget"`, …).
    pub declared: Vec<(String, &'static str)>,
    /// Uses not yet matched to a declaration: name and position.
    pub used: Vec<(String, u32)>,
}

/// Whether an expression is read where `in` is no operator: the head
/// of a `for`, and of a `for await`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForInit {
    No,
    Yes,
    Await,
}

impl ForInit {
    pub(crate) fn is_set(self) -> bool {
        self != ForInit::No
    }
}

/// The statement a sub-statement is the body of. acorn passes a string
/// and only asks whether it is there, `"if"` or `"label"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StatementContext {
    If,
    Label,
    /// A loop, `with`, or a label on top of another context.
    Other,
}

pub(crate) struct Parser {
    /// The input as UTF-16 code units, the way JavaScript indexes it.
    pub input: Vec<u16>,
    pub pos: usize,
    pub ty: Tok,
    /// The current token's word, string value or operator.
    pub value: String,
    /// The current string token holds a lone surrogate.
    pub value_lone_surrogate: bool,
    pub start: usize,
    pub end: usize,
    pub last_tok_start: usize,
    pub last_tok_end: usize,
    pub context: Vec<Ctx>,
    pub expr_allowed: bool,
    pub contains_esc: bool,
    pub in_template_element: bool,
    pub strict: bool,
    pub potential_arrow_at: i64,
    pub potential_arrow_in_for_await: bool,
    /// Positions of the first `yield` / `await` expression and `await`
    /// name in the parameters being read. Zero is unset.
    pub yield_pos: usize,
    pub await_pos: usize,
    pub await_ident_pos: usize,
    pub labels: Vec<Label>,
    /// Names exported before any declaration of them was seen, in the
    /// order met.
    pub undefined_exports: Vec<(String, u32)>,
    /// The names the module exports so far.
    pub exports: Vec<String>,
    pub scope_stack: Vec<Scope>,
    pub private_name_stack: Vec<PrivateNames>,
    /// How deep expressions are nested, and statements. acorn's stack
    /// runs out by expression nesting whatever the statement around it.
    pub depth: u32,
    pub statement_depth: u32,
    /// How long the chain of prefix operators being read is.
    pub unary_depth: u32,
    /// Keep a parenthesized expression as a node of its own. The
    /// compiler asks for that reading an expression, not a script.
    pub preserve_parens: bool,
    /// A component's script may export what the component declares
    /// elsewhere: the compiler empties the list of undeclared exports
    /// after every statement.
    pub forget_undefined_exports: bool,
}

/// Deeper nesting than this is reported the way acorn reports running
/// out of stack. Where acorn itself gives up is up to the JavaScript
/// engine it runs in and varies from run to run; this is about where
/// Node 24 does for nested arrays.
pub(crate) const MAX_DEPTH: u32 = 984;

/// Runs one level of the parser's recursion with room on the stack:
/// more is taken from the heap when little is left.
pub(crate) fn with_stack<T>(f: impl FnOnce() -> T) -> T {
    stacker::maybe_grow(128 * 1024, 4 * 1024 * 1024, f)
}

impl Parser {
    pub(crate) fn new(input: Vec<u16>, start_pos: usize) -> Parser {
        let mut parser = Parser {
            input,
            pos: start_pos,
            ty: Tok::Eof,
            value: String::new(),
            value_lone_surrogate: false,
            start: start_pos,
            end: start_pos,
            last_tok_start: start_pos,
            last_tok_end: start_pos,
            context: vec![Ctx::BStat],
            expr_allowed: true,
            contains_esc: false,
            in_template_element: false,
            // Module code is strict from its first token.
            strict: true,
            potential_arrow_at: -1,
            potential_arrow_in_for_await: false,
            yield_pos: 0,
            await_pos: 0,
            await_ident_pos: 0,
            labels: Vec::new(),
            undefined_exports: Vec::new(),
            exports: Vec::new(),
            scope_stack: Vec::new(),
            private_name_stack: Vec::new(),
            depth: 0,
            statement_depth: 0,
            unary_depth: 0,
            preserve_parens: false,
            forget_undefined_exports: false,
        };
        parser.enter_scope(SCOPE_TOP);
        parser
    }

    // --- location.js ---

    pub(crate) fn raise<T>(&self, pos: usize, message: impl Into<String>) -> PResult<T> {
        Err(Stop::Error(Raised {
            pos: pos as u32,
            message: message.into(),
        }))
    }

    pub(crate) fn unexpected<T>(&self) -> PResult<T> {
        self.raise(self.start, "Unexpected token")
    }

    pub(crate) fn unexpected_at<T>(&self, pos: usize) -> PResult<T> {
        self.raise(pos, "Unexpected token")
    }

    /// Guards one level of expression nesting.
    pub(crate) fn descend(&mut self) -> PResult<()> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return self.raise(self.start, "Not enough stack space to parse input");
        }
        Ok(())
    }

    pub(crate) fn ascend(&mut self) {
        self.depth -= 1;
    }

    /// Guards one level of statement nesting.
    pub(crate) fn descend_statement(&mut self) -> PResult<()> {
        self.statement_depth += 1;
        if self.statement_depth > MAX_DEPTH {
            return self.raise(self.start, "Not enough stack space to parse input");
        }
        Ok(())
    }

    pub(crate) fn ascend_statement(&mut self) {
        self.statement_depth -= 1;
    }

    // --- input access ---

    /// The code unit at `i`, or `-1` past the end (JavaScript's `NaN`).
    pub(crate) fn code_at(&self, i: usize) -> i32 {
        self.input.get(i).map_or(-1, |&c| i32::from(c))
    }

    pub(crate) fn slice(&self, from: usize, to: usize) -> String {
        let to = to.min(self.input.len());
        let from = from.min(to);
        String::from_utf16_lossy(&self.input[from..to])
    }

    pub(crate) fn slice_is(&self, from: usize, to: usize, text: &str) -> bool {
        let to = to.min(self.input.len());
        from <= to && self.input[from..to].iter().copied().eq(text.encode_utf16())
    }

    /// Whether a line break lies in `from..to`.
    pub(crate) fn line_break_between(&self, from: usize, to: usize) -> bool {
        let to = to.min(self.input.len());
        from < to
            && self.input[from..to]
                .iter()
                .any(|&c| is_new_line(i32::from(c)))
    }

    /// The length of what `skipWhiteSpace` matches at `from`: white
    /// space and whole comments.
    pub(crate) fn skip_white_space_len(&self, from: usize) -> usize {
        let len = self.input.len();
        let mut i = from;
        loop {
            let c = self.code_at(i);
            if is_regex_white_space(c) {
                i += 1;
            } else if c == 47 && self.code_at(i + 1) == 47 {
                i += 2;
                while i < len && !is_new_line(self.code_at(i)) {
                    i += 1;
                }
            } else if c == 47 && self.code_at(i + 1) == 42 {
                let mut close = i + 2;
                while close + 1 < len && !(self.input[close] == 42 && self.input[close + 1] == 47) {
                    close += 1;
                }
                if close + 1 >= len {
                    break;
                }
                i = close + 2;
            } else {
                break;
            }
        }
        i - from
    }

    // --- scope.js ---

    pub(crate) fn enter_scope(&mut self, flags: u32) {
        self.scope_stack.push(Scope {
            flags,
            var: Vec::new(),
            lexical: Vec::new(),
            functions: Vec::new(),
        });
    }

    pub(crate) fn exit_scope(&mut self) {
        self.scope_stack.pop();
    }

    fn treat_functions_as_var_in(flags: u32) -> bool {
        flags & SCOPE_FUNCTION != 0
    }

    pub(crate) fn treat_functions_as_var(&self) -> bool {
        Self::treat_functions_as_var_in(self.current_scope().flags)
    }

    pub(crate) fn declare_name(&mut self, name: &str, binding: Bind, pos: u32) -> PResult<()> {
        let mut redeclared = false;
        let has = |list: &[String]| list.iter().any(|n| n == name);
        match binding {
            Bind::Lexical => {
                let scope = self.current_scope_mut();
                redeclared = has(&scope.lexical) || has(&scope.functions) || has(&scope.var);
                scope.lexical.push(name.to_string());
                if scope.flags & SCOPE_TOP != 0 {
                    self.undefined_exports.retain(|(n, _)| n != name);
                }
            }
            Bind::SimpleCatch => {
                self.current_scope_mut().lexical.push(name.to_string());
            }
            Bind::Function => {
                let as_var = self.treat_functions_as_var();
                let scope = self.current_scope_mut();
                redeclared = if as_var {
                    has(&scope.lexical)
                } else {
                    has(&scope.lexical) || has(&scope.var)
                };
                scope.functions.push(name.to_string());
            }
            _ => {
                for i in (0..self.scope_stack.len()).rev() {
                    let scope = &mut self.scope_stack[i];
                    let simple_catch_parameter = scope.flags & SCOPE_SIMPLE_CATCH != 0
                        && scope.lexical.first().is_some_and(|n| n == name);
                    if (has(&scope.lexical) && !simple_catch_parameter)
                        || (!Self::treat_functions_as_var_in(scope.flags) && has(&scope.functions))
                    {
                        redeclared = true;
                        break;
                    }
                    scope.var.push(name.to_string());
                    let flags = scope.flags;
                    if flags & SCOPE_TOP != 0 {
                        self.undefined_exports.retain(|(n, _)| n != name);
                    }
                    if flags & SCOPE_VAR != 0 {
                        break;
                    }
                }
            }
        }
        if redeclared {
            return self.raise(
                pos as usize,
                format!("Identifier '{name}' has already been declared"),
            );
        }
        Ok(())
    }

    pub(crate) fn check_local_export(&mut self, name: &str, start: u32) {
        let top = &self.scope_stack[0];
        let declared = top.lexical.iter().any(|n| n == name) || top.var.iter().any(|n| n == name);
        if !declared {
            match self.undefined_exports.iter_mut().find(|(n, _)| n == name) {
                Some(entry) => entry.1 = start,
                None => self.undefined_exports.push((name.to_string(), start)),
            }
        }
    }

    pub(crate) fn current_scope(&self) -> &Scope {
        &self.scope_stack[self.scope_stack.len() - 1]
    }

    fn current_scope_mut(&mut self) -> &mut Scope {
        let last = self.scope_stack.len() - 1;
        &mut self.scope_stack[last]
    }

    pub(crate) fn current_var_scope(&self) -> &Scope {
        let wanted = SCOPE_VAR | SCOPE_CLASS_FIELD_INIT | SCOPE_CLASS_STATIC_BLOCK;
        self.scope_stack
            .iter()
            .rev()
            .find(|scope| scope.flags & wanted != 0)
            .unwrap_or(&self.scope_stack[0])
    }

    pub(crate) fn current_this_scope(&self) -> &Scope {
        let wanted = SCOPE_VAR | SCOPE_CLASS_FIELD_INIT | SCOPE_CLASS_STATIC_BLOCK;
        self.scope_stack
            .iter()
            .rev()
            .find(|scope| scope.flags & wanted != 0 && scope.flags & SCOPE_ARROW == 0)
            .unwrap_or(&self.scope_stack[0])
    }

    // --- state.js getters ---

    pub(crate) fn in_function(&self) -> bool {
        self.current_var_scope().flags & SCOPE_FUNCTION != 0
    }

    pub(crate) fn in_generator(&self) -> bool {
        self.current_var_scope().flags & SCOPE_GENERATOR != 0
    }

    pub(crate) fn in_async(&self) -> bool {
        self.current_var_scope().flags & SCOPE_ASYNC != 0
    }

    pub(crate) fn can_await(&self) -> bool {
        for scope in self.scope_stack.iter().rev() {
            if scope.flags & (SCOPE_CLASS_STATIC_BLOCK | SCOPE_CLASS_FIELD_INIT) != 0 {
                return false;
            }
            if scope.flags & SCOPE_FUNCTION != 0 {
                return scope.flags & SCOPE_ASYNC != 0;
            }
        }
        // A module's top level.
        true
    }

    pub(crate) fn allow_super(&self) -> bool {
        self.current_this_scope().flags & SCOPE_SUPER != 0
    }

    pub(crate) fn allow_direct_super(&self) -> bool {
        self.current_this_scope().flags & SCOPE_DIRECT_SUPER != 0
    }

    pub(crate) fn allow_new_dot_target(&self) -> bool {
        self.scope_stack.iter().rev().any(|scope| {
            scope.flags & (SCOPE_CLASS_STATIC_BLOCK | SCOPE_CLASS_FIELD_INIT) != 0
                || (scope.flags & SCOPE_FUNCTION != 0 && scope.flags & SCOPE_ARROW == 0)
        })
    }

    pub(crate) fn in_class_static_block(&self) -> bool {
        self.current_var_scope().flags & SCOPE_CLASS_STATIC_BLOCK != 0
    }

    // --- node.js ---

    pub(crate) fn start_node(&self) -> Node {
        Node::at(self.start as u32)
    }

    pub(crate) fn start_node_at(&self, pos: usize) -> Node {
        Node::at(pos as u32)
    }

    pub(crate) fn finish_node(&self, mut node: Node, ty: Ty) -> Node {
        node.ty = ty;
        node.end = self.last_tok_end as u32;
        node
    }

    // --- parseutil.js ---

    /// Whether the code at `start` begins with a `"use strict"`
    /// directive.
    pub(crate) fn strict_directive(&self, mut start: usize) -> bool {
        loop {
            start += self.skip_white_space_len(start);
            let Some((length, value_is_use_strict)) = self.string_literal_at(start) else {
                return false;
            };
            if value_is_use_strict {
                let space_start = start + length;
                let end = space_start + self.skip_white_space_len(space_start);
                let next = self.code_at(end);
                if next == 59 || next == 125 {
                    return true;
                }
                let continues = matches!(
                    u8::try_from(next).unwrap_or(0),
                    b'(' | b'`'
                        | b'.'
                        | b'['
                        | b'+'
                        | b'-'
                        | b'/'
                        | b'*'
                        | b'%'
                        | b'<'
                        | b'>'
                        | b'='
                        | b','
                        | b'?'
                        | b'^'
                        | b'&'
                ) || (next == 33 && self.code_at(end + 1) == 61)
                    || (next == 105 && self.keyword_op_at(end));
                return self.line_break_between(space_start, end) && !continues;
            }
            start += length;
            start += self.skip_white_space_len(start);
            if self.code_at(start) == 59 {
                start += 1;
            }
        }
    }

    /// The string literal at `start`, as the directive check reads it:
    /// its length, and whether its text is exactly `use strict`.
    fn string_literal_at(&self, start: usize) -> Option<(usize, bool)> {
        let quote = self.code_at(start);
        if quote != 34 && quote != 39 {
            return None;
        }
        let mut i = start + 1;
        loop {
            let c = self.code_at(i);
            if c < 0 {
                return None;
            }
            if c == quote {
                break;
            }
            if c == 92 {
                if self.code_at(i + 1) < 0 {
                    return None;
                }
                i += 2;
            } else {
                i += 1;
            }
        }
        Some((i + 1 - start, self.slice_is(start + 1, i, "use strict")))
    }

    fn keyword_op_at(&self, pos: usize) -> bool {
        let stop = self.input.len().min(pos + 11);
        let mut end = pos + 1;
        while end < stop {
            let ch = self.full_char_code_at(end);
            if !is_identifier_char(ch) {
                break;
            }
            end += if ch <= 0xffff { 1 } else { 2 };
        }
        (end == pos + 2 && self.slice_is(pos, end, "in"))
            || (end == pos + 10 && self.slice_is(pos, end, "instanceof"))
    }

    pub(crate) fn eat(&mut self, ty: Tok) -> PResult<bool> {
        if self.ty == ty {
            self.next()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub(crate) fn is_contextual(&self, name: &str) -> bool {
        self.ty == Tok::Name && self.value == name && !self.contains_esc
    }

    pub(crate) fn eat_contextual(&mut self, name: &str) -> PResult<bool> {
        if !self.is_contextual(name) {
            return Ok(false);
        }
        self.next()?;
        Ok(true)
    }

    pub(crate) fn expect_contextual(&mut self, name: &str) -> PResult<()> {
        if !self.eat_contextual(name)? {
            return self.unexpected();
        }
        Ok(())
    }

    pub(crate) fn can_insert_semicolon(&self) -> bool {
        self.ty == Tok::Eof
            || self.ty == Tok::BraceR
            || self.line_break_between(self.last_tok_end, self.start)
    }

    pub(crate) fn semicolon(&mut self) -> PResult<()> {
        if !self.eat(Tok::Semi)? && !self.can_insert_semicolon() {
            return self.unexpected();
        }
        Ok(())
    }

    pub(crate) fn after_trailing_comma(&mut self, ty: Tok, not_next: bool) -> PResult<bool> {
        if self.ty == ty {
            if !not_next {
                self.next()?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    pub(crate) fn expect(&mut self, ty: Tok) -> PResult<()> {
        if !self.eat(ty)? {
            return self.unexpected();
        }
        Ok(())
    }

    pub(crate) fn check_pattern_errors(
        &self,
        errors: Option<&DestructuringErrors>,
        is_assign: bool,
    ) -> PResult<()> {
        let Some(errors) = errors else {
            return Ok(());
        };
        if errors.trailing_comma > -1 {
            return self.raise(
                errors.trailing_comma as usize,
                "Comma is not permitted after the rest element",
            );
        }
        let parens = if is_assign {
            errors.parenthesized_assign
        } else {
            errors.parenthesized_bind
        };
        if parens > -1 {
            return self.raise(
                parens as usize,
                if is_assign {
                    "Assigning to rvalue"
                } else {
                    "Parenthesized pattern"
                },
            );
        }
        Ok(())
    }

    /// `checkExpressionErrors(errors)`: whether there are any.
    pub(crate) fn has_expression_errors(&self, errors: Option<&DestructuringErrors>) -> bool {
        errors.is_some_and(|e| e.shorthand_assign >= 0 || e.double_proto >= 0)
    }

    /// `checkExpressionErrors(errors, true)`: throws them.
    pub(crate) fn throw_expression_errors(
        &self,
        errors: Option<&DestructuringErrors>,
    ) -> PResult<()> {
        let Some(errors) = errors else {
            return Ok(());
        };
        if errors.shorthand_assign >= 0 {
            return self.raise(
                errors.shorthand_assign as usize,
                "Shorthand property assignments are valid only in destructuring patterns",
            );
        }
        if errors.double_proto >= 0 {
            return self.raise(
                errors.double_proto as usize,
                "Redefinition of __proto__ property",
            );
        }
        Ok(())
    }

    pub(crate) fn check_yield_await_in_default_params(&self) -> PResult<()> {
        if self.yield_pos != 0 && (self.await_pos == 0 || self.yield_pos < self.await_pos) {
            return self.raise(self.yield_pos, "Yield expression cannot be a default value");
        }
        if self.await_pos != 0 {
            return self.raise(self.await_pos, "Await expression cannot be a default value");
        }
        Ok(())
    }

    pub(crate) fn is_simple_assign_target(expr: &Node) -> bool {
        if expr.is(Ty::ParenthesizedExpression) {
            return Self::is_simple_assign_target(expr.first());
        }
        expr.is(Ty::Identifier) || expr.is(Ty::MemberExpression)
    }
}

pub(crate) fn is_new_line(code: i32) -> bool {
    code == 10 || code == 13 || code == 0x2028 || code == 0x2029
}

/// What `\s` matches in a JavaScript regular expression.
fn is_regex_white_space(code: i32) -> bool {
    matches!(
        code,
        9..=13 | 32 | 0xa0 | 0x1680 | 0x2000..=0x200a | 0x2028 | 0x2029 | 0x202f | 0x205f | 0x3000
            | 0xfeff
    )
}
