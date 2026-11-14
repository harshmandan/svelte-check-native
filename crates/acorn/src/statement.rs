//! acorn's statement parser (`statement.js`).

use crate::identifier::{is_identifier_char, is_identifier_start};
use crate::state::{
    Bind, DestructuringErrors, ForInit, Label, LabelKind, Node, PResult, Parser, PrivateNames,
    SCOPE_CLASS_FIELD_INIT, SCOPE_CLASS_STATIC_BLOCK, SCOPE_SIMPLE_CATCH, SCOPE_SUPER,
    SCOPE_SWITCH, StatementContext, Ty, function_flags, with_stack,
};
use crate::token::Tok;

pub(crate) const FUNC_STATEMENT: u8 = 1;
pub(crate) const FUNC_HANGING_STATEMENT: u8 = 2;
pub(crate) const FUNC_NULLABLE_ID: u8 = 4;

/// Where a class stands, which decides whether it needs a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ClassPosition {
    Statement,
    Expression,
    /// After `export default`: a declaration that may go unnamed.
    NullableId,
}

pub(crate) fn check_key_name(node: &Node, name: &str) -> bool {
    let key = node.first();
    !node.computed
        && ((key.is(Ty::Identifier) && key.name == name)
            || (key.is(Ty::Literal) && key.string.as_deref() == Some(name)))
}

/// Whether a private name is declared twice in a class body. A getter
/// and a setter of the same staticness share a name.
pub(crate) fn is_private_name_conflicted(
    names: &mut Vec<(String, &'static str)>,
    element: &Node,
) -> bool {
    let name = &element.first().name;
    let next =
        if element.is(Ty::MethodDefinition) && (element.name == "get" || element.name == "set") {
            match (element.is_static, element.name.as_str()) {
                (true, "get") => "sget",
                (true, _) => "sset",
                (false, "get") => "iget",
                (false, _) => "iset",
            }
        } else {
            "true"
        };
    let Some(entry) = names.iter_mut().find(|(n, _)| n == name) else {
        names.push((name.clone(), next));
        return false;
    };
    let pair = matches!(
        (entry.1, next),
        ("iget", "iset") | ("iset", "iget") | ("sget", "sset") | ("sset", "sget")
    );
    if pair {
        entry.1 = "true";
        return false;
    }
    true
}

impl Parser {
    /// A whole program.
    pub(crate) fn parse_top_level(&mut self) -> PResult<()> {
        while self.ty != Tok::Eof {
            self.parse_statement(None, true)?;
        }
        if let Some((name, start)) = self.undefined_exports.first() {
            return self.raise(*start as usize, format!("Export '{name}' is not defined"));
        }
        self.next()
    }

    /// Whether the `let` the parser stands on starts a declaration.
    fn is_let(&self, context: Option<StatementContext>) -> bool {
        if !self.is_contextual("let") {
            return false;
        }
        let mut next = self.pos + self.skip_white_space_len(self.pos);
        let mut next_ch = self.full_char_code_at(next);
        if next_ch == 91 || next_ch == 92 {
            return true;
        }
        if context.is_some() {
            return false;
        }
        if next_ch == 123 {
            return true;
        }
        if is_identifier_start(next_ch) {
            let start = next;
            loop {
                next += if next_ch <= 0xffff { 1 } else { 2 };
                next_ch = self.full_char_code_at(next);
                if !is_identifier_char(next_ch) {
                    break;
                }
            }
            if next_ch == 92 {
                return true;
            }
            let word = self.slice(start, next);
            if word != "in" && word != "instanceof" {
                return true;
            }
        }
        false
    }

    /// Whether `async function` follows, on one line.
    fn is_async_function(&self) -> bool {
        if !self.is_contextual("async") {
            return false;
        }
        let next = self.pos + self.skip_white_space_len(self.pos);
        if self.line_break_between(self.pos, next) || !self.slice_is(next, next + 8, "function") {
            return false;
        }
        if next + 8 == self.input.len() {
            return true;
        }
        let after = self.full_char_code_at(next + 8);
        !(is_identifier_char(after) || after == 92)
    }

    /// A statement. `context` is the statement it is the body of;
    /// `top_level` is where `import` and `export` may stand.
    pub(crate) fn parse_statement(
        &mut self,
        context: Option<StatementContext>,
        top_level: bool,
    ) -> PResult<Node> {
        self.descend_statement()?;
        let result = with_stack(|| self.parse_statement_inner(context, top_level));
        self.ascend_statement();
        if result.is_ok() && self.forget_undefined_exports {
            if !self.undefined_exports.is_empty() {
                self.undefined_exports = std::rc::Rc::default();
            }
        }
        result
    }

    fn parse_statement_inner(
        &mut self,
        context: Option<StatementContext>,
        top_level: bool,
    ) -> PResult<Node> {
        if self.ts
            && let Some(node) = self.ts_parse_statement_start()?
        {
            return Ok(node);
        }
        let mut start_type = self.ty;
        let node = self.start_node();
        let mut kind: Option<String> = None;

        if self.is_let(context) {
            start_type = Tok::KwVar;
            kind = Some("let".to_string());
        }

        match start_type {
            Tok::KwBreak | Tok::KwContinue => {
                self.parse_break_continue_statement(node, start_type == Tok::KwBreak)
            }
            Tok::KwDebugger => {
                self.next()?;
                self.semicolon()?;
                Ok(self.finish_node(node, Ty::Other))
            }
            Tok::KwDo => self.parse_do_statement(node),
            Tok::KwFor => self.parse_for_statement(node),
            Tok::KwFunction => {
                // Strict code has no function declaration as the body
                // of another statement.
                let allowed_in_sloppy = matches!(
                    context,
                    Some(StatementContext::If | StatementContext::Label)
                );
                if context.is_some() && (self.strict || !allowed_in_sloppy) {
                    return self.unexpected();
                }
                self.parse_function_statement(node, false, context.is_none())
            }
            Tok::KwClass => {
                if context.is_some() {
                    return self.unexpected();
                }
                self.parse_class(node, ClassPosition::Statement)
            }
            Tok::KwIf => self.parse_if_statement(node),
            Tok::KwReturn => self.parse_return_statement(node),
            Tok::KwSwitch => self.parse_switch_statement(node),
            Tok::KwThrow => self.parse_throw_statement(node),
            Tok::KwTry => self.parse_try_statement(node),
            Tok::KwConst | Tok::KwVar => {
                let kind = kind.unwrap_or_else(|| self.value.clone());
                if context.is_some() && kind != "var" {
                    return self.unexpected();
                }
                self.parse_var_statement(node, &kind, false)
            }
            Tok::KwWhile => self.parse_while_statement(node),
            Tok::KwWith => self.parse_with_statement(node),
            Tok::BraceL => self.parse_block(true, Some(node), false),
            Tok::Semi => {
                self.next()?;
                Ok(self.finish_node(node, Ty::Other))
            }
            Tok::KwExport | Tok::KwImport => {
                if start_type == Tok::KwImport {
                    let next = self.pos + self.skip_white_space_len(self.pos);
                    let next_ch = self.code_at(next);
                    // `import(` and `import.` are expressions.
                    if next_ch == 40 || next_ch == 46 {
                        let expr = self.parse_expression(ForInit::No, None)?;
                        return self.parse_expression_statement(node, expr);
                    }
                }
                if !top_level {
                    return self.raise(
                        self.start,
                        "'import' and 'export' may only appear at the top level",
                    );
                }
                if start_type == Tok::KwImport {
                    if self.ts {
                        self.ts_parse_import(node)
                    } else {
                        self.parse_import(node)
                    }
                } else if self.ts {
                    self.ts_parse_export(node)
                } else {
                    self.parse_export(node)
                }
            }
            _ => {
                if self.is_async_function() {
                    if context.is_some() {
                        return self.unexpected();
                    }
                    self.next()?;
                    return self.parse_function_statement(node, true, context.is_none());
                }
                let maybe_name = self.value.clone();
                let expr = self.parse_expression(ForInit::No, None)?;
                if start_type == Tok::Name && expr.is(Ty::Identifier) && self.eat(Tok::Colon)? {
                    self.parse_labeled_statement(node, maybe_name, &expr, context)
                } else {
                    self.parse_expression_statement(node, expr)
                }
            }
        }
    }

    fn parse_break_continue_statement(&mut self, node: Node, is_break: bool) -> PResult<Node> {
        self.next()?;
        let label = if self.eat(Tok::Semi)? || self.can_insert_semicolon() {
            None
        } else if self.ty != Tok::Name {
            return self.unexpected();
        } else {
            let label = self.parse_ident(false)?;
            self.semicolon()?;
            Some(label.name)
        };

        // The label or enclosing statement this jumps out of.
        let found = self.labels.iter().any(|lab| {
            if label.is_none() || lab.name == label {
                if lab.kind.is_some() && (is_break || lab.kind == Some(LabelKind::Loop)) {
                    return true;
                }
                if label.is_some() && is_break {
                    return true;
                }
            }
            false
        });
        if !found {
            return self.raise(
                node.start as usize,
                if is_break {
                    "Unsyntactic break"
                } else {
                    "Unsyntactic continue"
                },
            );
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    fn loop_label() -> Label {
        Label {
            kind: Some(LabelKind::Loop),
            name: None,
            statement_start: None,
        }
    }

    fn parse_do_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.labels.push(Self::loop_label());
        self.parse_statement(Some(StatementContext::Other), false)?;
        self.labels.pop();
        self.expect(Tok::KwWhile)?;
        self.parse_paren_expression()?;
        self.eat(Tok::Semi)?;
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_for_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        let await_at = if self.can_await() && self.eat_contextual("await")? {
            Some(self.last_tok_start)
        } else {
            None
        };
        self.labels.push(Self::loop_label());
        self.enter_scope(0);
        self.expect(Tok::ParenL)?;
        if self.ty == Tok::Semi {
            if let Some(await_at) = await_at {
                return self.unexpected_at(await_at);
            }
            return self.parse_for(node);
        }
        let is_let = self.is_let(None);
        if self.ty == Tok::KwVar || self.ty == Tok::KwConst || is_let {
            let mut init = self.start_node();
            let kind = if is_let {
                "let".to_string()
            } else {
                self.value.clone()
            };
            self.next()?;
            self.parse_var(&mut init, true, &kind, false)?;
            let init = self.finish_node(init, Ty::VariableDeclaration);
            return self.parse_for_after_init(node, &init, await_at);
        }
        let starts_with_let = self.is_contextual("let");
        let mut is_for_of = false;
        let contains_esc = self.contains_esc;
        let mut errors = DestructuringErrors::new();
        let init_pos = self.start;
        let mut init = if await_at.is_some() {
            self.parse_expr_subscripts(Some(&mut errors), ForInit::Await)?
        } else {
            self.parse_expression(ForInit::Yes, Some(&mut errors))?
        };
        if self.ty == Tok::KwIn || {
            is_for_of = self.is_contextual("of");
            is_for_of
        } {
            if let Some(await_at) = await_at {
                if self.ty == Tok::KwIn {
                    return self.unexpected_at(await_at);
                }
            } else if is_for_of
                && init.start as usize == init_pos
                && !contains_esc
                && init.is(Ty::Identifier)
                && init.name == "async"
            {
                return self.unexpected();
            }
            if starts_with_let && is_for_of {
                return self.raise(
                    init.start as usize,
                    "The left-hand side of a for-of loop may not start with 'let'.",
                );
            }
            self.to_assignable(&mut init, false, Some(&errors))?;
            self.check_lval_pattern(&init, Bind::None, None)?;
            return self.parse_for_in(node, &init);
        }
        self.throw_expression_errors(Some(&errors))?;
        if let Some(await_at) = await_at {
            return self.unexpected_at(await_at);
        }
        self.parse_for(node)
    }

    /// The rest of a `for` whose head starts with a declaration.
    fn parse_for_after_init(
        &mut self,
        node: Node,
        init: &Node,
        await_at: Option<usize>,
    ) -> PResult<Node> {
        if (self.ty == Tok::KwIn || self.is_contextual("of")) && init.list.len() == 1 {
            if self.ty == Tok::KwIn
                && let Some(await_at) = await_at
            {
                return self.unexpected_at(await_at);
            }
            return self.parse_for_in(node, init);
        }
        if let Some(await_at) = await_at {
            return self.unexpected_at(await_at);
        }
        self.parse_for(node)
    }

    fn parse_function_statement(
        &mut self,
        node: Node,
        is_async: bool,
        declaration_position: bool,
    ) -> PResult<Node> {
        self.next()?;
        let hanging = if declaration_position {
            0
        } else {
            FUNC_HANGING_STATEMENT
        };
        self.parse_function(node, FUNC_STATEMENT | hanging, false, is_async, ForInit::No)
    }

    fn parse_if_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.parse_paren_expression()?;
        self.parse_statement(Some(StatementContext::If), false)?;
        if self.eat(Tok::KwElse)? {
            self.parse_statement(Some(StatementContext::If), false)?;
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_return_statement(&mut self, node: Node) -> PResult<Node> {
        if !self.in_function() {
            return self.raise(self.start, "'return' outside of function");
        }
        self.next()?;
        if !(self.eat(Tok::Semi)? || self.can_insert_semicolon()) {
            self.parse_expression(ForInit::No, None)?;
            self.semicolon()?;
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_switch_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.parse_paren_expression()?;
        self.expect(Tok::BraceL)?;
        self.labels.push(Label {
            kind: Some(LabelKind::Switch),
            name: None,
            statement_start: None,
        });
        self.enter_scope(SCOPE_SWITCH);

        let mut in_case = false;
        let mut saw_default = false;
        while self.ty != Tok::BraceR {
            if self.ty == Tok::KwCase || self.ty == Tok::KwDefault {
                let is_case = self.ty == Tok::KwCase;
                in_case = true;
                self.next()?;
                if is_case {
                    self.parse_expression(ForInit::No, None)?;
                } else {
                    if saw_default {
                        return self.raise(self.last_tok_start, "Multiple default clauses");
                    }
                    saw_default = true;
                }
                self.expect(Tok::Colon)?;
            } else {
                if !in_case {
                    return self.unexpected();
                }
                self.parse_statement(None, false)?;
            }
        }
        self.exit_scope();
        self.next()?;
        self.labels.pop();
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_throw_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        if self.line_break_between(self.last_tok_end, self.start) {
            return self.raise(self.last_tok_end, "Illegal newline after throw");
        }
        self.parse_expression(ForInit::No, None)?;
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_catch_clause_param(&mut self) -> PResult<()> {
        let param = self.parse_binding_atom()?;
        let simple = param.is(Ty::Identifier);
        self.enter_scope(if simple { SCOPE_SIMPLE_CATCH } else { 0 });
        self.check_lval_pattern(
            &param,
            if simple {
                Bind::SimpleCatch
            } else {
                Bind::Lexical
            },
            None,
        )?;
        if self.ts {
            self.ts_try_parse_type_annotation()?;
        }
        self.expect(Tok::ParenR)
    }

    fn parse_try_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.parse_block(true, None, false)?;
        let mut handler = false;
        if self.ty == Tok::KwCatch {
            self.next()?;
            if self.eat(Tok::ParenL)? {
                self.parse_catch_clause_param()?;
            } else {
                self.enter_scope(0);
            }
            self.parse_block(false, None, false)?;
            self.exit_scope();
            handler = true;
        }
        let finalizer = if self.eat(Tok::KwFinally)? {
            self.parse_block(true, None, false)?;
            true
        } else {
            false
        };
        if !handler && !finalizer {
            return self.raise(node.start as usize, "Missing catch or finally clause");
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    pub(crate) fn parse_var_statement(
        &mut self,
        mut node: Node,
        kind: &str,
        allow_missing_initializer: bool,
    ) -> PResult<Node> {
        if self.ts {
            return self.ts_parse_var_statement(node, kind, allow_missing_initializer);
        }
        self.next()?;
        self.parse_var(&mut node, false, kind, allow_missing_initializer)?;
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::VariableDeclaration))
    }

    fn parse_while_statement(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.parse_paren_expression()?;
        self.labels.push(Self::loop_label());
        self.parse_statement(Some(StatementContext::Other), false)?;
        self.labels.pop();
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_with_statement(&mut self, node: Node) -> PResult<Node> {
        if self.strict {
            return self.raise(self.start, "'with' in strict mode");
        }
        self.next()?;
        self.parse_paren_expression()?;
        self.parse_statement(Some(StatementContext::Other), false)?;
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_labeled_statement(
        &mut self,
        node: Node,
        maybe_name: String,
        expr: &Node,
        context: Option<StatementContext>,
    ) -> PResult<Node> {
        if self
            .labels
            .iter()
            .any(|label| label.name.as_deref() == Some(&maybe_name))
        {
            return self.raise(
                expr.start as usize,
                format!("Label '{maybe_name}' is already declared"),
            );
        }
        let kind = if self.ty.is_loop() {
            Some(LabelKind::Loop)
        } else if self.ty == Tok::KwSwitch {
            Some(LabelKind::Switch)
        } else {
            None
        };
        // Labels stacked on this statement all label what follows.
        let start = self.start as u32;
        for label in self.labels.iter_mut().rev() {
            if label.statement_start == Some(node.start) {
                label.statement_start = Some(start);
                label.kind = kind;
            } else {
                break;
            }
        }
        self.labels.push(Label {
            name: Some(maybe_name),
            kind,
            statement_start: Some(start),
        });
        // A label on the body of an `if` or a loop is neither a plain
        // `if` body nor a plain labelled one.
        let body_context = match context {
            None | Some(StatementContext::Label) => StatementContext::Label,
            Some(_) => StatementContext::Other,
        };
        self.parse_statement(Some(body_context), false)?;
        self.labels.pop();
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_expression_statement(&mut self, mut node: Node, expr: Node) -> PResult<Node> {
        if self.ts && expr.is(Ty::Identifier) {
            match self.ts_parse_expression_statement(node, &expr.name)? {
                Ok(declaration) => return Ok(declaration),
                Err(back) => node = back,
            }
        }
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::Other))
    }

    /// A block. A function's body shares the function's scope, and
    /// ends the strict mode its own directive began.
    pub(crate) fn parse_block(
        &mut self,
        create_new_lexical_scope: bool,
        node: Option<Node>,
        exit_strict: bool,
    ) -> PResult<Node> {
        let node = node.unwrap_or_else(|| self.start_node());
        self.expect(Tok::BraceL)?;
        if create_new_lexical_scope {
            self.enter_scope(0);
        }
        while self.ty != Tok::BraceR {
            self.parse_statement(None, false)?;
        }
        if exit_strict {
            self.strict = false;
        }
        self.next()?;
        if create_new_lexical_scope {
            self.exit_scope();
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    /// The rest of a `for (init; test; update)` after its `init`.
    fn parse_for(&mut self, node: Node) -> PResult<Node> {
        self.expect(Tok::Semi)?;
        if self.ty != Tok::Semi {
            self.parse_expression(ForInit::No, None)?;
        }
        self.expect(Tok::Semi)?;
        if self.ty != Tok::ParenR {
            self.parse_expression(ForInit::No, None)?;
        }
        self.expect(Tok::ParenR)?;
        self.parse_statement(Some(StatementContext::Other), false)?;
        self.exit_scope();
        self.labels.pop();
        Ok(self.finish_node(node, Ty::Other))
    }

    /// The rest of a `for (left in right)` or `for (left of right)`.
    fn parse_for_in(&mut self, node: Node, init: &Node) -> PResult<Node> {
        let is_for_in = self.ty == Tok::KwIn;
        self.next()?;

        // Only sloppy `for (var x = 1 in y)` keeps an initializer.
        if init.is(Ty::VariableDeclaration)
            && let Some(Some(declarator)) = init.list.first()
            && declarator.b.is_some()
            && (!is_for_in
                || self.strict
                || init.name != "var"
                || !declarator.first().is(Ty::Identifier))
        {
            return self.raise(
                init.start as usize,
                format!(
                    "{} loop variable declaration may not have an initializer",
                    if is_for_in { "for-in" } else { "for-of" }
                ),
            );
        }
        if is_for_in {
            self.parse_expression(ForInit::No, None)?;
        } else {
            self.parse_maybe_assign(ForInit::No, None)?;
        }
        self.expect(Tok::ParenR)?;
        self.parse_statement(Some(StatementContext::Other), false)?;
        self.exit_scope();
        self.labels.pop();
        Ok(self.finish_node(node, Ty::Other))
    }

    /// The declarators of a `var`, `let` or `const`, into `node`.
    pub(crate) fn parse_var(
        &mut self,
        node: &mut Node,
        is_for: bool,
        kind: &str,
        allow_missing_initializer: bool,
    ) -> PResult<()> {
        node.name = kind.to_string();
        loop {
            let mut decl = self.start_node();
            let mut id = self.parse_binding_atom()?;
            self.check_lval_pattern(
                &id,
                if kind == "var" {
                    Bind::Var
                } else {
                    Bind::Lexical
                },
                None,
            )?;
            if self.ts {
                self.ts_after_var_id(&mut id)?;
            }
            let id_is_name = id.is(Ty::Identifier);
            decl.a = Some(Box::new(id));
            let in_or_of = self.ty == Tok::KwIn || self.is_contextual("of");
            if self.eat(Tok::Eq)? {
                let for_init = if is_for { ForInit::Yes } else { ForInit::No };
                decl.b = Some(Box::new(self.parse_maybe_assign(for_init, None)?));
            } else if !allow_missing_initializer && kind == "const" && !in_or_of {
                return self.unexpected();
            } else if !allow_missing_initializer && !id_is_name && !(is_for && in_or_of) {
                return self.raise(
                    self.last_tok_end,
                    "Complex binding patterns require an initialization value",
                );
            }
            node.list
                .push(Some(self.finish_node(decl, Ty::VariableDeclarator)));
            if !self.eat(Tok::Comma)? {
                break;
            }
        }
        Ok(())
    }

    /// A function, from after `function`. `statement` says whether it
    /// is a declaration, one standing as another statement's body, or
    /// one that may go unnamed.
    pub(crate) fn parse_function(
        &mut self,
        mut node: Node,
        statement: u8,
        allow_expression_body: bool,
        is_async: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        if self.ts {
            return self.ts_parse_function(
                node,
                statement,
                allow_expression_body,
                is_async,
                for_init,
            );
        }
        if self.ty == Tok::Star && statement & FUNC_HANGING_STATEMENT != 0 {
            return self.unexpected();
        }
        let generator = self.eat(Tok::Star)?;

        if statement & FUNC_STATEMENT != 0 {
            let id = if statement & FUNC_NULLABLE_ID != 0 && self.ty != Tok::Name {
                None
            } else {
                Some(self.parse_ident(false)?)
            };
            if let Some(id) = &id
                && statement & FUNC_HANGING_STATEMENT == 0
            {
                let binding = if self.strict || generator || is_async {
                    if self.treat_functions_as_var() {
                        Bind::Var
                    } else {
                        Bind::Lexical
                    }
                } else {
                    Bind::Function
                };
                self.check_lval_simple(id, binding, None)?;
            }
            node.a = id.map(Box::new);
        }

        let old_yield_pos = self.yield_pos;
        let old_await_pos = self.await_pos;
        let old_await_ident_pos = self.await_ident_pos;
        self.yield_pos = 0;
        self.await_pos = 0;
        self.await_ident_pos = 0;
        self.enter_scope(function_flags(is_async, generator));

        if statement & FUNC_STATEMENT == 0 && self.ty == Tok::Name {
            node.a = Some(Box::new(self.parse_ident(false)?));
        }

        self.expect(Tok::ParenL)?;
        node.list = self.parse_binding_list(Tok::ParenR, false, true, None)?;
        self.check_yield_await_in_default_params()?;
        self.parse_function_body(
            &mut node,
            allow_expression_body,
            false,
            for_init,
            crate::ts_expression::BodyOf::Other,
        )?;

        self.yield_pos = old_yield_pos;
        self.await_pos = old_await_pos;
        self.await_ident_pos = old_await_ident_pos;
        Ok(self.finish_node(
            node,
            if statement & FUNC_STATEMENT != 0 {
                Ty::FunctionDeclaration
            } else {
                Ty::FunctionExpression
            },
        ))
    }

    /// A class, from `class`.
    pub(crate) fn parse_class(&mut self, mut node: Node, position: ClassPosition) -> PResult<Node> {
        if self.ts {
            return self.ts_parse_class(node, position);
        }
        self.next()?;

        // A class is strict code throughout.
        let old_strict = self.strict;
        self.strict = true;

        if self.ty == Tok::Name {
            let id = self.parse_ident(false)?;
            if position != ClassPosition::Expression {
                self.check_lval_simple(&id, Bind::Lexical, None)?;
            }
            node.a = Some(Box::new(id));
        } else if position == ClassPosition::Statement {
            return self.unexpected();
        }

        let has_super_class = if self.eat(Tok::KwExtends)? {
            self.parse_expr_subscripts(None, ForInit::No)?;
            true
        } else {
            false
        };

        self.private_name_stack.push(PrivateNames {
            declared: Vec::new(),
            used: Vec::new(),
        });
        let mut had_constructor = false;
        self.expect(Tok::BraceL)?;
        while self.ty != Tok::BraceR {
            let Some(element) = self.parse_class_element(has_super_class)? else {
                continue;
            };
            if element.is(Ty::MethodDefinition) && element.name == "constructor" {
                if had_constructor {
                    return self.raise(
                        element.start as usize,
                        "Duplicate constructor in the same class",
                    );
                }
                had_constructor = true;
            } else if element.a.is_some() && element.first().is(Ty::PrivateIdentifier) {
                let conflicted = match self.private_name_stack.last_mut() {
                    Some(names) => is_private_name_conflicted(&mut names.declared, &element),
                    None => false,
                };
                if conflicted {
                    return self.raise(
                        element.first().start as usize,
                        format!(
                            "Identifier '#{}' has already been declared",
                            element.first().name
                        ),
                    );
                }
            }
        }
        self.strict = old_strict;
        self.next()?;
        self.exit_class_body()?;
        Ok(self.finish_node(
            node,
            if position == ClassPosition::Expression {
                Ty::Other
            } else {
                Ty::ClassDeclaration
            },
        ))
    }

    fn parse_class_element(&mut self, constructor_allows_super: bool) -> PResult<Option<Node>> {
        if self.eat(Tok::Semi)? {
            return Ok(None);
        }

        let mut node = self.start_node();
        let mut key_name = String::new();
        let mut is_generator = false;
        let mut is_async = false;
        let mut kind = "method".to_string();
        let mut is_static = false;

        if self.eat_contextual("static")? {
            if self.eat(Tok::BraceL)? {
                self.parse_class_static_block()?;
                return Ok(Some(self.finish_node(node, Ty::StaticBlock)));
            }
            if self.is_class_element_name_start() || self.ty == Tok::Star {
                is_static = true;
            } else {
                key_name = "static".to_string();
            }
        }
        node.is_static = is_static;
        if key_name.is_empty() && self.eat_contextual("async")? {
            if (self.is_class_element_name_start() || self.ty == Tok::Star)
                && !self.can_insert_semicolon()
            {
                is_async = true;
            } else {
                key_name = "async".to_string();
            }
        }
        if key_name.is_empty() && self.eat(Tok::Star)? {
            is_generator = true;
        }
        if key_name.is_empty() && !is_async && !is_generator {
            let last_value = self.value.clone();
            if self.eat_contextual("get")? || self.eat_contextual("set")? {
                if self.is_class_element_name_start() {
                    kind = last_value;
                } else {
                    key_name = last_value;
                }
            }
        }

        if key_name.is_empty() {
            self.parse_class_element_name(&mut node)?;
        } else {
            // `static`, `async`, `get` or `set` was the name itself.
            node.computed = false;
            let mut key = self.start_node_at(self.last_tok_start);
            key.name = key_name;
            node.a = Some(Box::new(self.finish_node(key, Ty::Identifier)));
        }

        if self.ty == Tok::ParenL || kind != "method" || is_generator || is_async {
            let is_constructor = !node.is_static && check_key_name(&node, "constructor");
            let allows_direct_super = is_constructor && constructor_allows_super;
            if is_constructor && kind != "method" {
                return self.raise(
                    node.first().start as usize,
                    "Constructor can't have get/set modifier",
                );
            }
            node.name = if is_constructor {
                "constructor".to_string()
            } else {
                kind
            };
            self.parse_class_method(node, is_generator, is_async, allows_direct_super)
                .map(Some)
        } else {
            self.parse_class_field(node).map(Some)
        }
    }

    pub(crate) fn is_class_element_name_start(&self) -> bool {
        if self.ts && self.ty.is_identifier() {
            return true;
        }
        matches!(
            self.ty,
            Tok::Name | Tok::PrivateId | Tok::Num | Tok::String | Tok::BracketL
        ) || self.ty.keyword().is_some()
    }

    pub(crate) fn parse_class_element_name(&mut self, element: &mut Node) -> PResult<()> {
        if self.ty == Tok::PrivateId {
            if self.value == "constructor" {
                return self.raise(
                    self.start,
                    "Classes can't have an element named '#constructor'",
                );
            }
            element.computed = false;
            element.a = Some(Box::new(self.parse_private_ident()?));
            Ok(())
        } else {
            self.parse_property_name(element)
        }
    }

    fn parse_class_method(
        &mut self,
        mut method: Node,
        is_generator: bool,
        is_async: bool,
        allows_direct_super: bool,
    ) -> PResult<Node> {
        let key_start = method.first().start as usize;
        if method.name == "constructor" {
            if is_generator {
                return self.raise(key_start, "Constructor can't be a generator");
            }
            if is_async {
                return self.raise(key_start, "Constructor can't be an async method");
            }
        } else if method.is_static && check_key_name(&method, "prototype") {
            return self.raise(
                key_start,
                "Classes may not have a static property named prototype",
            );
        }

        let value = self.parse_method(is_generator, is_async, allows_direct_super)?;
        if method.name == "get" && !value.list.is_empty() {
            return self.raise(value.start as usize, "getter should have no params");
        }
        if method.name == "set" && value.list.len() != 1 {
            return self.raise(value.start as usize, "setter should have exactly one param");
        }
        if method.name == "set"
            && let Some(Some(parameter)) = value.list.first()
            && parameter.is(Ty::RestElement)
        {
            return self.raise(parameter.start as usize, "Setter cannot use rest params");
        }
        method.b = Some(Box::new(value));
        Ok(self.finish_node(method, Ty::MethodDefinition))
    }

    pub(crate) fn parse_class_field(&mut self, field: Node) -> PResult<Node> {
        if check_key_name(&field, "constructor") {
            return self.raise(
                field.first().start as usize,
                "Classes can't have a field named 'constructor'",
            );
        } else if field.is_static && check_key_name(&field, "prototype") {
            return self.raise(
                field.first().start as usize,
                "Classes can't have a static field named 'prototype'",
            );
        }

        if self.eat(Tok::Eq)? {
            // The initializer is read like a method's body.
            self.enter_scope(SCOPE_CLASS_FIELD_INIT | SCOPE_SUPER);
            self.parse_maybe_assign(ForInit::No, None)?;
            self.exit_scope();
        }
        self.semicolon()?;
        Ok(self.finish_node(field, Ty::PropertyDefinition))
    }

    pub(crate) fn parse_class_static_block(&mut self) -> PResult<()> {
        let old_labels = std::mem::take(&mut self.labels);
        self.enter_scope(SCOPE_CLASS_STATIC_BLOCK | SCOPE_SUPER);
        while self.ty != Tok::BraceR {
            self.parse_statement(None, false)?;
        }
        self.next()?;
        self.exit_scope();
        self.labels = old_labels;
        Ok(())
    }

    /// Leaves a class body: a private name used in it and declared
    /// neither there nor in an enclosing class is an error.
    pub(crate) fn exit_class_body(&mut self) -> PResult<()> {
        let Some(PrivateNames { declared, used }) = self.private_name_stack.pop() else {
            return Ok(());
        };
        for (name, start) in used {
            if declared.iter().any(|(n, _)| *n == name) {
                continue;
            }
            match self.private_name_stack.last_mut() {
                Some(parent) => parent.used.push((name, start)),
                None => {
                    return self.raise(
                        start as usize,
                        format!("Private field '#{name}' must be declared in an enclosing class"),
                    );
                }
            }
        }
        Ok(())
    }

    fn parse_export_all_declaration(&mut self, node: Node) -> PResult<Node> {
        if self.eat_contextual("as")? {
            let exported = self.parse_module_export_name()?;
            self.check_export(export_name(&exported), self.last_tok_start)?;
        }
        self.expect_contextual("from")?;
        if self.ty != Tok::String {
            return self.unexpected();
        }
        self.parse_expr_atom(None, ForInit::No, false)?;
        self.parse_with_clause()?;
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_export(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        // export * from '...'
        if self.eat(Tok::Star)? {
            return self.parse_export_all_declaration(node);
        }
        if self.eat(Tok::KwDefault)? {
            self.check_export("default", self.last_tok_start)?;
            self.parse_export_default_declaration()?;
            return Ok(self.finish_node(node, Ty::Other));
        }
        // export var|const|let|function|class ...
        if self.should_parse_export_statement() {
            let declaration = self.parse_statement(None, false)?;
            if declaration.is(Ty::VariableDeclaration) {
                for declarator in declaration.list.iter().flatten() {
                    self.check_pattern_export(declarator.first())?;
                }
            } else {
                let id = declaration.first();
                self.check_export(&id.name, id.start as usize)?;
            }
        } else {
            // export { x, y as z } [from '...']
            let specifiers = self.parse_export_specifiers()?;
            if self.eat_contextual("from")? {
                if self.ty != Tok::String {
                    return self.unexpected();
                }
                self.parse_expr_atom(None, ForInit::No, false)?;
                self.parse_with_clause()?;
            } else {
                for specifier in &specifiers {
                    let local = specifier.first();
                    // A local binding is a name; only a re-export may
                    // name it with a string.
                    self.check_unreserved(local.start as usize, &local.name)?;
                    self.check_local_export(&local.name, local.start);
                    if local.is(Ty::Literal) {
                        return self.raise(
                            local.start as usize,
                            "A string literal cannot be used as an exported binding without `from`.",
                        );
                    }
                }
            }
            self.semicolon()?;
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    pub(crate) fn parse_export_default_declaration(&mut self) -> PResult<()> {
        let is_async = self.ty != Tok::KwFunction && self.is_async_function();
        if self.ty == Tok::KwFunction || is_async {
            let node = self.start_node();
            self.next()?;
            if is_async {
                self.next()?;
            }
            self.parse_function(
                node,
                FUNC_STATEMENT | FUNC_NULLABLE_ID,
                false,
                is_async,
                ForInit::No,
            )?;
        } else if self.ty == Tok::KwClass {
            let node = self.start_node();
            self.parse_class(node, ClassPosition::NullableId)?;
        } else {
            self.parse_maybe_assign(ForInit::No, None)?;
            self.semicolon()?;
        }
        Ok(())
    }

    fn check_export(&mut self, name: &str, pos: usize) -> PResult<()> {
        // acorn-typescript keeps the names and never looks at them.
        if self.ts {
            return Ok(());
        }
        if self.exports.iter().any(|n| n == name) {
            return self.raise(pos, format!("Duplicate export '{name}'"));
        }
        crate::state::add_name(&mut self.exports, name);
        Ok(())
    }

    fn check_pattern_export(&mut self, pattern: &Node) -> PResult<()> {
        match pattern.ty {
            Ty::Identifier => self.check_export(&pattern.name, pattern.start as usize),
            Ty::ObjectPattern | Ty::ArrayPattern => {
                for element in pattern.list.iter().flatten() {
                    self.check_pattern_export(element)?;
                }
                Ok(())
            }
            Ty::Property => self.check_pattern_export(pattern.second()),
            Ty::AssignmentPattern | Ty::RestElement => self.check_pattern_export(pattern.first()),
            _ => Ok(()),
        }
    }

    pub(crate) fn should_parse_export_statement(&self) -> bool {
        if self.ts && (self.ty.is_ts_declaration_start() || self.ty == Tok::At) {
            return true;
        }
        matches!(
            self.ty,
            Tok::KwVar | Tok::KwConst | Tok::KwClass | Tok::KwFunction
        ) || self.is_let(None)
            || self.is_async_function()
    }

    fn parse_export_specifiers(&mut self) -> PResult<Vec<Node>> {
        let mut nodes = Vec::new();
        let mut first = true;
        // export { x, y as z } [from '...']
        self.expect(Tok::BraceL)?;
        while !self.eat(Tok::BraceR)? {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
                if self.after_trailing_comma(Tok::BraceR, false)? {
                    break;
                }
            }
            let mut node = self.start_node();
            let local = self.parse_module_export_name()?;
            let exported = if self.eat_contextual("as")? {
                self.parse_module_export_name()?
            } else {
                local.clone()
            };
            self.check_export(export_name(&exported), exported.start as usize)?;
            node.a = Some(Box::new(local));
            node.b = Some(Box::new(exported));
            nodes.push(self.finish_node(node, Ty::ExportSpecifier));
        }
        Ok(nodes)
    }

    fn parse_import(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        // import '...'
        if self.ty == Tok::String {
            self.parse_expr_atom(None, ForInit::No, false)?;
        } else {
            self.parse_import_specifiers()?;
            self.expect_contextual("from")?;
            if self.ty != Tok::String {
                return self.unexpected();
            }
            self.parse_expr_atom(None, ForInit::No, false)?;
        }
        self.parse_with_clause()?;
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_import_specifiers(&mut self) -> PResult<()> {
        let mut first = true;
        if self.ty == Tok::Name {
            // import defaultObj, { x, y as z } from '...'
            let local = self.parse_ident(false)?;
            self.check_lval_simple(&local, Bind::Lexical, None)?;
            if !self.eat(Tok::Comma)? {
                return Ok(());
            }
        }
        if self.ty == Tok::Star {
            self.next()?;
            self.expect_contextual("as")?;
            let local = self.parse_ident(false)?;
            return self.check_lval_simple(&local, Bind::Lexical, None);
        }
        self.expect(Tok::BraceL)?;
        while !self.eat(Tok::BraceR)? {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
                if self.after_trailing_comma(Tok::BraceR, false)? {
                    break;
                }
            }
            let imported = self.parse_module_export_name()?;
            let local = if self.eat_contextual("as")? {
                self.parse_ident(false)?
            } else {
                self.check_unreserved(imported.start as usize, &imported.name)?;
                imported
            };
            self.check_lval_simple(&local, Bind::Lexical, None)?;
        }
        Ok(())
    }

    /// The `with { type: "json" }` of an import or re-export.
    fn parse_with_clause(&mut self) -> PResult<()> {
        if !self.eat(Tok::KwWith)? {
            return Ok(());
        }
        self.expect(Tok::BraceL)?;
        let mut keys: Vec<String> = Vec::new();
        let mut first = true;
        while !self.eat(Tok::BraceR)? {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
                if self.after_trailing_comma(Tok::BraceR, false)? {
                    break;
                }
            }
            let key = if self.ty == Tok::String {
                self.parse_expr_atom(None, ForInit::No, false)?
            } else {
                self.parse_ident(true)?
            };
            self.expect(Tok::Colon)?;
            if self.ty != Tok::String {
                return self.unexpected();
            }
            self.parse_expr_atom(None, ForInit::No, false)?;
            let key_name = export_name(&key).to_string();
            if keys.contains(&key_name) {
                return self.raise(
                    key.start as usize,
                    format!("Duplicate attribute key '{key_name}'"),
                );
            }
            keys.push(key_name);
        }
        Ok(())
    }

    /// The name an import or export goes by: a word, or a string.
    pub(crate) fn parse_module_export_name(&mut self) -> PResult<Node> {
        if self.ty == Tok::String {
            let lone_surrogate = self.value_lone_surrogate;
            let literal = self.parse_literal()?;
            if lone_surrogate {
                return self.raise(
                    literal.start as usize,
                    "An export name cannot include a lone surrogate.",
                );
            }
            return Ok(literal);
        }
        self.parse_ident(true)
    }
}

/// The text of a name that is either a word or a string literal.
fn export_name(node: &Node) -> &str {
    if node.is(Ty::Identifier) {
        &node.name
    } else {
        node.string.as_deref().unwrap_or("")
    }
}
