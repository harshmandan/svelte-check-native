//! acorn's expression parser (`expression.js`).

use crate::identifier::{is_keyword, is_reserved, is_reserved_strict};
use crate::state::{
    Bind, DestructuringErrors, ForInit, Lit, Nest, Node, PResult, Parser, SCOPE_ARROW,
    SCOPE_DIRECT_SUPER, SCOPE_SUPER, SCOPE_VAR, Ty, function_flags, with_stack,
};
use crate::token::{Ctx, Tok};
use crate::ts_expression::{BodyOf, SubscriptStart};

fn is_local_variable_access(node: &Node) -> bool {
    node.is(Ty::Identifier)
        || (node.is(Ty::ParenthesizedExpression) && is_local_variable_access(node.first()))
}

fn is_private_field_access(node: &Node) -> bool {
    (node.is(Ty::MemberExpression) && node.second().is(Ty::PrivateIdentifier))
        || ((node.is(Ty::ChainExpression) || node.is(Ty::ParenthesizedExpression))
            && is_private_field_access(node.first()))
}

impl Parser {
    /// A second `__proto__: value` in an object literal. `proto` says
    /// whether the object has one already.
    fn check_prop_clash(
        &self,
        prop: &Node,
        proto: &mut bool,
        errors: Option<&mut DestructuringErrors>,
    ) -> PResult<()> {
        if prop.is(Ty::SpreadElement) || prop.computed || prop.method || prop.shorthand {
            return Ok(());
        }
        let key = prop.first();
        let name = match key.ty {
            Ty::Identifier => key.name.as_str(),
            Ty::Literal => key.string.as_deref().unwrap_or(""),
            _ => return Ok(()),
        };
        if name == "__proto__" && prop.name == "init" {
            if *proto {
                match errors {
                    Some(errors) => {
                        if errors.double_proto < 0 {
                            errors.double_proto = i64::from(key.start);
                        }
                    }
                    None => {
                        return self
                            .raise(key.start as usize, "Redefinition of __proto__ property");
                    }
                }
            }
            *proto = true;
        }
        Ok(())
    }

    /// An expression, with the comma operator.
    pub(crate) fn parse_expression(
        &mut self,
        for_init: ForInit,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        let start_pos = self.start;
        let expr = self.parse_maybe_assign(for_init, errors.as_deref_mut())?;
        if self.ty == Tok::Comma {
            let node = self.start_node_at(start_pos);
            while self.eat(Tok::Comma)? {
                self.parse_maybe_assign(for_init, errors.as_deref_mut())?;
            }
            return Ok(self.finish_node(node, Ty::Other));
        }
        Ok(expr)
    }

    /// An assignment, or anything that binds tighter.
    pub(crate) fn parse_maybe_assign(
        &mut self,
        for_init: ForInit,
        errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        self.parse_maybe_assign_item(for_init, errors, false)
    }

    /// `parseMaybeAssign`. `paren_item` is acorn's `afterLeftParse`
    /// hook, which only ever is `parseParenItem`: the expression is an
    /// item of what may be an arrow function's parameter list.
    pub(crate) fn parse_maybe_assign_item(
        &mut self,
        for_init: ForInit,
        errors: Option<&mut DestructuringErrors>,
        paren_item: bool,
    ) -> PResult<Node> {
        self.descend()?;
        let result = with_stack(|| {
            if self.ts {
                self.ts_parse_maybe_assign(for_init, errors, paren_item)
            } else {
                self.parse_maybe_assign_inner(for_init, errors)
            }
        });
        self.ascend();
        result
    }

    fn parse_maybe_assign_inner(
        &mut self,
        for_init: ForInit,
        errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        if self.is_contextual("yield") {
            if self.in_generator() {
                return self.parse_yield(for_init);
            }
            self.expr_allowed = false;
        }

        let own = errors.is_none();
        let mut own_errors = DestructuringErrors::new();
        let mut old_paren_assign = -1;
        let mut old_trailing_comma = -1;
        let mut old_double_proto = -1;
        let errors = match errors {
            Some(errors) => {
                old_paren_assign = errors.parenthesized_assign;
                old_trailing_comma = errors.trailing_comma;
                old_double_proto = errors.double_proto;
                errors.parenthesized_assign = -1;
                errors.trailing_comma = -1;
                errors
            }
            None => &mut own_errors,
        };

        let start_pos = self.start;
        if self.ty == Tok::ParenL || self.ty == Tok::Name {
            self.potential_arrow_at = start_pos as i64;
            self.potential_arrow_in_for_await = for_init == ForInit::Await;
        }
        let mut left = self.parse_maybe_conditional(for_init, Some(errors))?;
        if self.ty.is_assign() {
            let mut node = self.start_node_at(start_pos);
            node.name = self.value.clone();
            let is_eq = self.ty == Tok::Eq;
            if is_eq {
                self.to_assignable(&mut left, false, Some(errors))?;
            }
            if !own {
                errors.parenthesized_assign = -1;
                errors.trailing_comma = -1;
                if errors.shorthand_assign >= i64::from(left.start) {
                    errors.shorthand_assign = -1;
                }
                if errors.double_proto >= i64::from(left.start) {
                    errors.double_proto = -1;
                }
            }
            if is_eq {
                self.check_lval_pattern(&left, Bind::None, None)?;
            } else {
                self.check_lval_simple(&left, Bind::None, None)?;
            }
            node.a = Some(Box::new(left));
            self.next()?;
            self.chain_link = true;
            node.b = Some(Box::new(self.parse_maybe_assign(for_init, None)?));
            if old_double_proto > -1 {
                errors.double_proto = old_double_proto;
            }
            return Ok(self.finish_node(node, Ty::AssignmentExpression));
        } else if own {
            self.throw_expression_errors(Some(errors))?;
        }
        if old_paren_assign > -1 {
            errors.parenthesized_assign = old_paren_assign;
        }
        if old_trailing_comma > -1 {
            errors.trailing_comma = old_trailing_comma;
        }
        Ok(left)
    }

    pub(crate) fn parse_maybe_conditional(
        &mut self,
        for_init: ForInit,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        if self.ts {
            return self.ts_parse_maybe_conditional(for_init, errors);
        }
        let start_pos = self.start;
        let expr = self.parse_expr_ops(for_init, errors.as_deref_mut())?;
        if self.has_expression_errors(errors.as_deref()) {
            return Ok(expr);
        }
        let is_arrow_here =
            expr.is(Ty::ArrowFunctionExpression) && expr.start as usize == start_pos;
        if !is_arrow_here && self.eat(Tok::Question)? {
            let node = self.start_node_at(start_pos);
            self.parse_maybe_assign(ForInit::No, None)?;
            self.expect(Tok::Colon)?;
            self.chain_link = true;
            self.parse_maybe_assign(for_init, None)?;
            return Ok(self.finish_node(node, Ty::Other));
        }
        Ok(expr)
    }

    pub(crate) fn parse_expr_ops(
        &mut self,
        for_init: ForInit,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        let start_pos = self.start;
        let expr = self.parse_maybe_unary(errors.as_deref_mut(), false, false, for_init)?;
        if self.has_expression_errors(errors.as_deref()) {
            return Ok(expr);
        }
        if expr.start as usize == start_pos && expr.is(Ty::ArrowFunctionExpression) {
            return Ok(expr);
        }
        self.parse_expr_op(expr, start_pos, -1, for_init)
    }

    /// Binary operators of precedence above `min_prec`, after `left`.
    fn parse_expr_op(
        &mut self,
        mut left: Node,
        left_start: usize,
        min_prec: i32,
        for_init: ForInit,
    ) -> PResult<Node> {
        let mut operators = 0;
        loop {
            operators += 1;
            if operators > crate::state::MAX_BINARY_CHAIN {
                return self.stack_exhausted();
            }
            if self.ts {
                match self.ts_parse_as_or_satisfies(left, left_start, min_prec, for_init)? {
                    Ok(node) => {
                        left = node;
                        continue;
                    }
                    Err(back) => left = back,
                }
            }
            let Some(mut prec) = self.ty.binop() else {
                return Ok(left);
            };
            if (for_init.is_set() && self.ty == Tok::KwIn) || prec <= min_prec {
                return Ok(left);
            }
            let logical = self.ty == Tok::LogicalOr || self.ty == Tok::LogicalAnd;
            let coalesce = self.ty == Tok::Coalesce;
            if coalesce {
                // Above `||` and `&&`, so that neither is read into its
                // right side.
                prec = 2;
            }
            self.next()?;
            let start_pos = self.start;
            let operand = self.parse_maybe_unary(None, false, false, for_init)?;
            let right = self.parse_expr_op(operand, start_pos, prec, for_init)?;
            let node = self.build_binary(left_start, &right)?;
            if (logical && self.ty == Tok::Coalesce)
                || (coalesce && (self.ty == Tok::LogicalOr || self.ty == Tok::LogicalAnd))
            {
                return self.raise(
                    self.start,
                    "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
                );
            }
            left = node;
        }
    }

    pub(crate) fn build_binary(&mut self, start_pos: usize, right: &Node) -> PResult<Node> {
        if right.is(Ty::PrivateIdentifier) {
            return self.raise(
                right.start as usize,
                "Private identifier can only be left side of binary expression",
            );
        }
        let node = self.start_node_at(start_pos);
        Ok(self.finish_node(node, Ty::Other))
    }

    /// A unary operator and its operand, `await`, a postfix `++`, `**`.
    pub(crate) fn parse_maybe_unary(
        &mut self,
        errors: Option<&mut DestructuringErrors>,
        saw_unary: bool,
        inc_dec: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        // Nesting is counted where the enclosing expression is read; a
        // run of prefix operators nests without one.
        self.unary_depth += 1;
        if self.unary_depth > crate::state::MAX_UNARY_DEPTH {
            return self.stack_exhausted();
        }
        let result = with_stack(|| {
            if self.ts && self.ts_match_left_relational() {
                self.ts_parse_type_assertion()
            } else {
                self.parse_maybe_unary_inner(errors, saw_unary, inc_dec, for_init)
            }
        });
        self.unary_depth -= 1;
        result
    }

    fn parse_maybe_unary_inner(
        &mut self,
        mut errors: Option<&mut DestructuringErrors>,
        mut saw_unary: bool,
        inc_dec: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        let start_pos = self.start;
        let mut expr;
        if self.is_contextual("await") && self.can_await() {
            expr = self.parse_await(for_init)?;
            saw_unary = true;
        } else if self.ty.prefix() {
            let node = self.start_node();
            let update = self.ty == Tok::IncDec;
            let operator = self.value.clone();
            self.next()?;
            let argument = self.parse_maybe_unary(None, true, update, for_init)?;
            self.throw_expression_errors(errors.as_deref())?;
            if update {
                self.check_lval_simple(&argument, Bind::None, None)?;
            } else if self.strict && operator == "delete" && is_local_variable_access(&argument) {
                return self.raise(
                    node.start as usize,
                    "Deleting local variable in strict mode",
                );
            } else if operator == "delete" && is_private_field_access(&argument) {
                return self.raise(node.start as usize, "Private fields can not be deleted");
            } else {
                saw_unary = true;
            }
            let mut node = node;
            if !update {
                node.name = operator;
                node.a = Some(Box::new(argument));
            }
            expr = self.finish_node(
                node,
                if update {
                    Ty::Other
                } else {
                    Ty::UnaryExpression
                },
            );
        } else if !saw_unary && self.ty == Tok::PrivateId {
            if for_init.is_set() || self.private_name_stack.is_empty() {
                return self.unexpected();
            }
            expr = self.parse_private_ident()?;
            if self.ty != Tok::KwIn {
                return self.unexpected();
            }
        } else {
            expr = self.parse_expr_subscripts(errors.as_deref_mut(), for_init)?;
            if self.has_expression_errors(errors.as_deref()) {
                return Ok(expr);
            }
            while self.ty.postfix() && !self.can_insert_semicolon() {
                let node = self.start_node_at(start_pos);
                self.check_lval_simple(&expr, Bind::None, None)?;
                self.next()?;
                expr = self.finish_node(node, Ty::Other);
            }
        }

        let is_arrow_here =
            expr.is(Ty::ArrowFunctionExpression) && expr.start as usize == start_pos;
        if !inc_dec && !is_arrow_here && self.eat(Tok::StarStar)? {
            if saw_unary {
                return self.unexpected_at(self.last_tok_start);
            }
            let right = self.parse_maybe_unary(None, false, false, for_init)?;
            return self.build_binary(start_pos, &right);
        }
        Ok(expr)
    }

    /// An atom and the member accesses and calls on it.
    pub(crate) fn parse_expr_subscripts(
        &mut self,
        mut errors: Option<&mut DestructuringErrors>,
        for_init: ForInit,
    ) -> PResult<Node> {
        let start_pos = self.start;
        let mut old_double_proto = -1;
        let mut old_shorthand_assign = -1;
        if let Some(errors) = errors.as_deref_mut() {
            old_double_proto = errors.double_proto;
            old_shorthand_assign = errors.shorthand_assign;
            errors.double_proto = -1;
            errors.shorthand_assign = -1;
        }
        let expr = self.parse_expr_atom(errors.as_deref_mut(), for_init, false)?;
        if expr.is(Ty::ArrowFunctionExpression)
            && !self.slice_is(self.last_tok_start, self.last_tok_end, ")")
        {
            return Ok(expr);
        }
        let expr_end = expr.end;
        let result = self.parse_subscripts(expr, start_pos, false, for_init)?;
        if let Some(errors) = errors {
            if result.end > expr_end {
                self.throw_expression_errors(Some(errors))?;
                let start = i64::from(result.start);
                if errors.parenthesized_assign >= start {
                    errors.parenthesized_assign = -1;
                }
                if errors.parenthesized_bind >= start {
                    errors.parenthesized_bind = -1;
                }
                if errors.trailing_comma >= start {
                    errors.trailing_comma = -1;
                }
            }
            if old_double_proto > -1 {
                errors.double_proto = old_double_proto;
            }
            if old_shorthand_assign > -1 {
                errors.shorthand_assign = old_shorthand_assign;
            }
        }
        Ok(result)
    }

    pub(crate) fn parse_subscripts(
        &mut self,
        mut base: Node,
        start_pos: usize,
        no_calls: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        let mut maybe_async_arrow = base.is(Ty::Identifier)
            && base.name == "async"
            && self.last_tok_end == base.end as usize
            && !self.can_insert_semicolon()
            && base.end - base.start == 5
            && self.potential_arrow_at == i64::from(base.start);
        let mut optional_chained = false;
        loop {
            let base_end = base.end;
            let element = self.parse_subscript(
                base,
                start_pos,
                no_calls,
                maybe_async_arrow,
                optional_chained,
                for_init,
            )?;
            if element.optional {
                optional_chained = true;
            }
            if element.end == base_end || element.is(Ty::ArrowFunctionExpression) {
                if optional_chained {
                    let mut chain = self.start_node_at(start_pos);
                    chain.a = Some(Box::new(element));
                    return Ok(self.finish_node(chain, Ty::ChainExpression));
                }
                return Ok(element);
            }
            base = element;
            maybe_async_arrow = false;
        }
    }

    fn parse_subscript(
        &mut self,
        mut base: Node,
        start_pos: usize,
        no_calls: bool,
        maybe_async_arrow: bool,
        mut optional_chained: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        if self.ts {
            match self.ts_parse_subscript_start(
                base,
                start_pos,
                no_calls,
                optional_chained,
                for_init,
            )? {
                SubscriptStart::Done(node) => return Ok(node),
                SubscriptStart::Continue {
                    base: same,
                    optional_chained: chained,
                } => {
                    base = same;
                    optional_chained = chained;
                }
            }
        }
        let optional = self.eat(Tok::QuestionDot)?;
        if no_calls && optional {
            return self.raise(
                self.last_tok_start,
                "Optional chaining cannot appear in the callee of new expressions",
            );
        }

        let computed = self.eat(Tok::BracketL)?;
        if computed
            || (optional && self.ty != Tok::ParenL && self.ty != Tok::BackQuote)
            || self.eat(Tok::Dot)?
        {
            let mut node = self.start_node_at(start_pos);
            let property = if computed {
                let property = self.parse_expression(ForInit::No, None)?;
                self.expect(Tok::BracketR)?;
                property
            } else if self.ty == Tok::PrivateId && !base.is(Ty::Super) {
                self.parse_private_ident()?
            } else {
                self.parse_ident(true)?
            };
            node.a = Some(Box::new(base));
            node.b = Some(Box::new(property));
            node.computed = computed;
            node.optional = optional;
            return Ok(self.finish_node(node, Ty::MemberExpression));
        }
        if !no_calls && self.eat(Tok::ParenL)? {
            let mut errors = DestructuringErrors::new();
            let old_yield_pos = self.yield_pos;
            let old_await_pos = self.await_pos;
            let old_await_ident_pos = self.await_ident_pos;
            self.yield_pos = 0;
            self.await_pos = 0;
            self.await_ident_pos = 0;
            // acorn-typescript: the arguments may be the parameters of
            // an `async` arrow function.
            let old_maybe_in_arrow_parameters = self.maybe_in_arrow_parameters;
            if self.ts {
                self.maybe_in_arrow_parameters = true;
            }
            let arguments = self.parse_expr_list(Tok::ParenR, true, false, Some(&mut errors))?;
            let is_async_arrow = maybe_async_arrow
                && !optional
                && if self.ts {
                    self.ts_should_parse_async_arrow()?
                } else {
                    !self.can_insert_semicolon() && self.eat(Tok::Arrow)?
                };
            if is_async_arrow {
                self.check_pattern_errors(Some(&errors), false)?;
                self.check_yield_await_in_default_params()?;
                if self.await_ident_pos > 0 && !self.ambient_allows_names() {
                    return self.raise(
                        self.await_ident_pos,
                        "Cannot use 'await' as identifier inside an async function",
                    );
                }
                self.yield_pos = old_yield_pos;
                self.await_pos = old_await_pos;
                self.await_ident_pos = old_await_ident_pos;
                let node = self.start_node_at(start_pos);
                let arrow = self.parse_arrow_expression(node, arguments, true, for_init)?;
                self.maybe_in_arrow_parameters = old_maybe_in_arrow_parameters;
                return Ok(arrow);
            }
            self.throw_expression_errors(Some(&errors))?;
            self.maybe_in_arrow_parameters = old_maybe_in_arrow_parameters;
            if old_yield_pos != 0 {
                self.yield_pos = old_yield_pos;
            }
            if old_await_pos != 0 {
                self.await_pos = old_await_pos;
            }
            if old_await_ident_pos != 0 {
                self.await_ident_pos = old_await_ident_pos;
            }
            let mut node = self.start_node_at(start_pos);
            node.optional = optional;
            return Ok(self.finish_node(node, Ty::Other));
        }
        if self.ty == Tok::BackQuote {
            if optional || optional_chained {
                return self.raise(
                    self.start,
                    "Optional chaining cannot appear in the tag of tagged template expressions",
                );
            }
            let node = self.start_node_at(start_pos);
            self.parse_template(true)?;
            return Ok(self.finish_node(node, Ty::Other));
        }
        Ok(base)
    }

    /// An atomic expression. `for_new` is the callee of `new`, where
    /// `import(…)` is no call.
    pub(crate) fn parse_expr_atom(
        &mut self,
        errors: Option<&mut DestructuringErrors>,
        for_init: ForInit,
        for_new: bool,
    ) -> PResult<Node> {
        // The tokenizer took a `/` for division; here it starts a
        // regular expression.
        if self.ty == Tok::Slash {
            self.read_regexp()?;
        }

        if self.ts && self.ty == Tok::At {
            self.parse_decorators(false)?;
            // `@dec abstract class {}` as an expression.
            if self.ts_is_abstract_class()? {
                let mut class = self.start_node();
                self.next()?;
                class.modifiers.push("abstract".to_string());
                return self.parse_class(class, crate::statement::ClassPosition::Expression);
            }
            return self.parse_expr_atom(errors, for_init, for_new);
        }

        let can_be_arrow = self.potential_arrow_at == self.start as i64;
        match self.ty {
            Tok::KwSuper => {
                if !self.allow_super() {
                    return self.raise(self.start, "'super' keyword outside a method");
                }
                let node = self.start_node();
                self.next()?;
                if self.ty == Tok::ParenL && !self.allow_direct_super() {
                    return self.raise(
                        node.start as usize,
                        "super() call outside constructor of a subclass",
                    );
                }
                if !matches!(self.ty, Tok::Dot | Tok::BracketL | Tok::ParenL) {
                    return self.unexpected();
                }
                Ok(self.finish_node(node, Ty::Super))
            }
            Tok::KwThis => {
                let node = self.start_node();
                self.next()?;
                Ok(self.finish_node(node, Ty::Other))
            }
            // In TypeScript any word acorn-typescript has a type for is
            // a name here too.
            ty if ty == Tok::Name || (self.ts && ty.is_ts_word()) => {
                let start_pos = self.start;
                let contains_esc = self.contains_esc;
                let mut id = self.parse_ident(false)?;
                if !contains_esc
                    && id.name == "async"
                    && !self.can_insert_semicolon()
                    && self.eat(Tok::KwFunction)?
                {
                    self.override_context(Ctx::FExpr);
                    let node = self.start_node_at(start_pos);
                    return self.parse_function(node, 0, false, true, for_init);
                }
                if can_be_arrow && !self.can_insert_semicolon() {
                    if self.eat(Tok::Arrow)? {
                        let node = self.start_node_at(start_pos);
                        return self.parse_arrow_expression(node, vec![Some(id)], false, for_init);
                    }
                    if id.name == "async"
                        && self.ty == Tok::Name
                        && !contains_esc
                        && (!self.potential_arrow_in_for_await
                            || self.value != "of"
                            || self.contains_esc)
                    {
                        id = self.parse_ident(false)?;
                        if self.can_insert_semicolon() || !self.eat(Tok::Arrow)? {
                            return self.unexpected();
                        }
                        let node = self.start_node_at(start_pos);
                        return self.parse_arrow_expression(node, vec![Some(id)], true, for_init);
                    }
                }
                Ok(id)
            }
            Tok::Regexp | Tok::Num | Tok::String => self.parse_literal(),
            Tok::KwNull | Tok::KwTrue | Tok::KwFalse => {
                let node = self.start_node();
                self.next()?;
                Ok(self.finish_node(node, Ty::Literal))
            }
            Tok::ParenL => {
                let start = self.start as i64;
                let expr = self.parse_paren_and_distinguish_expression(can_be_arrow, for_init)?;
                if let Some(errors) = errors {
                    if errors.parenthesized_assign < 0 && !Self::is_simple_assign_target(&expr) {
                        errors.parenthesized_assign = start;
                    }
                    if errors.parenthesized_bind < 0 {
                        errors.parenthesized_bind = start;
                    }
                }
                Ok(expr)
            }
            Tok::BracketL => {
                let mut node = self.start_node();
                self.next()?;
                node.list = self.parse_expr_list(Tok::BracketR, true, true, errors)?;
                Ok(self.finish_node(node, Ty::ArrayExpression))
            }
            Tok::BraceL => {
                self.override_context(Ctx::BExpr);
                self.parse_obj(false, errors)
            }
            Tok::KwFunction => {
                let node = self.start_node();
                self.next()?;
                self.parse_function(node, 0, false, false, ForInit::No)
            }
            Tok::KwClass => {
                let node = self.start_node();
                self.parse_class(node, crate::statement::ClassPosition::Expression)
            }
            Tok::KwNew => self.nested(Nest::Atom, |p| {
                if p.ts {
                    p.ts_parse_new()
                } else {
                    p.parse_new()
                }
            }),
            Tok::BackQuote => {
                let mut node = self.start_node();
                let substitutions = self.parse_template(false)?;
                node.list = vec![None; substitutions];
                Ok(self.finish_node(node, Ty::TemplateLiteral))
            }
            Tok::KwImport => self.parse_expr_import(for_new),
            _ => self.unexpected(),
        }
    }

    fn parse_expr_import(&mut self, for_new: bool) -> PResult<Node> {
        let node = self.start_node();
        if self.contains_esc {
            return self.raise(self.start, "Escape sequence in keyword import");
        }
        self.next()?;
        if self.ty == Tok::ParenL && !for_new {
            self.parse_dynamic_import(node)
        } else if self.ty == Tok::Dot {
            self.parse_import_meta(node)
        } else {
            self.unexpected()
        }
    }

    fn parse_dynamic_import(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.parse_maybe_assign(ForInit::No, None)?;
        if !self.eat(Tok::ParenR)? {
            self.expect(Tok::Comma)?;
            if !self.after_trailing_comma(Tok::ParenR, false)? {
                self.parse_maybe_assign(ForInit::No, None)?;
                if !self.eat(Tok::ParenR)? {
                    self.expect(Tok::Comma)?;
                    if !self.after_trailing_comma(Tok::ParenR, false)? {
                        return self.unexpected();
                    }
                }
            }
        }
        Ok(self.finish_node(node, Ty::ImportExpression))
    }

    fn parse_import_meta(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        let contains_esc = self.contains_esc;
        let property = self.parse_ident(true)?;
        if property.name != "meta" {
            return self.raise(
                property.start as usize,
                "The only valid meta property for import is 'import.meta'",
            );
        }
        if contains_esc {
            return self.raise(
                node.start as usize,
                "'import.meta' must not contain escaped characters",
            );
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    /// A number, string or regular expression literal.
    pub(crate) fn parse_literal(&mut self) -> PResult<Node> {
        let mut node = self.start_node();
        node.lit = match self.ty {
            Tok::String => {
                node.string = Some(self.value.clone());
                Lit::String
            }
            Tok::Num if self.end > 0 && self.code_at(self.end - 1) == 110 => Lit::BigInt,
            Tok::Num => Lit::Number,
            _ => Lit::Other,
        };
        self.next()?;
        Ok(self.finish_node(node, Ty::Literal))
    }

    pub(crate) fn parse_paren_expression(&mut self) -> PResult<Node> {
        self.expect(Tok::ParenL)?;
        let value = self.parse_expression(ForInit::No, None)?;
        self.expect(Tok::ParenR)?;
        Ok(value)
    }

    fn parse_paren_and_distinguish_expression(
        &mut self,
        can_be_arrow: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        let start_pos = self.start;
        // acorn-typescript: what follows may be parameters.
        let old_maybe_in_arrow_parameters = self.maybe_in_arrow_parameters;
        if self.ts {
            self.maybe_in_arrow_parameters = true;
        }
        self.next()?;

        let inner_start = self.start;
        let mut list: Vec<Option<Node>> = Vec::new();
        let mut first = true;
        let mut last_is_comma = false;
        let mut errors = DestructuringErrors::new();
        let old_yield_pos = self.yield_pos;
        let old_await_pos = self.await_pos;
        let mut spread_start = None;
        self.yield_pos = 0;
        self.await_pos = 0;
        while self.ty != Tok::ParenR {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
            }
            if self.after_trailing_comma(Tok::ParenR, true)? {
                last_is_comma = true;
                break;
            } else if self.ty == Tok::Ellipsis {
                spread_start = Some(self.start);
                let mut rest = self.parse_rest_binding()?;
                if self.ts {
                    rest = self.ts_parse_paren_item(rest)?;
                }
                list.push(Some(rest));
                if self.ty == Tok::Comma {
                    self.raise_comma_after_rest(self.start)?;
                }
                break;
            } else {
                let paren_item = self.ts;
                list.push(Some(self.parse_maybe_assign_item(
                    ForInit::No,
                    Some(&mut errors),
                    paren_item,
                )?));
            }
        }
        let inner_end = self.last_tok_end;
        self.expect(Tok::ParenR)?;
        self.maybe_in_arrow_parameters = old_maybe_in_arrow_parameters;

        let should_parse_arrow = can_be_arrow
            && if self.ts {
                self.ts_should_parse_arrow(&list)?
            } else {
                !self.can_insert_semicolon()
            };
        if should_parse_arrow && self.eat(Tok::Arrow)? {
            self.check_pattern_errors(Some(&errors), false)?;
            self.check_yield_await_in_default_params()?;
            self.yield_pos = old_yield_pos;
            self.await_pos = old_await_pos;
            let node = self.start_node_at(start_pos);
            return self.parse_arrow_expression(node, list, false, for_init);
        }

        if list.is_empty() || last_is_comma {
            return self.unexpected_at(self.last_tok_start);
        }
        if let Some(spread_start) = spread_start {
            return self.unexpected_at(spread_start);
        }
        self.throw_expression_errors(Some(&errors))?;
        if old_yield_pos != 0 {
            self.yield_pos = old_yield_pos;
        }
        if old_await_pos != 0 {
            self.await_pos = old_await_pos;
        }

        let value = if list.len() > 1 {
            let mut sequence = self.start_node_at(inner_start);
            sequence.end = inner_end as u32;
            sequence
        } else {
            match list.pop().flatten() {
                Some(only) => only,
                None => return self.unexpected(),
            }
        };
        if !self.preserve_parens {
            return Ok(value);
        }
        let mut parenthesized = self.start_node_at(start_pos);
        parenthesized.a = Some(Box::new(value));
        Ok(self.finish_node(parenthesized, Ty::ParenthesizedExpression))
    }

    fn parse_new(&mut self) -> PResult<Node> {
        if self.contains_esc {
            return self.raise(self.start, "Escape sequence in keyword new");
        }
        let node = self.start_node();
        self.next()?;
        if self.ty == Tok::Dot {
            self.next()?;
            let contains_esc = self.contains_esc;
            let property = self.parse_ident(true)?;
            if property.name != "target" {
                return self.raise(
                    property.start as usize,
                    "The only valid meta property for new is 'new.target'",
                );
            }
            if contains_esc {
                return self.raise(
                    node.start as usize,
                    "'new.target' must not contain escaped characters",
                );
            }
            if !self.allow_new_dot_target() {
                return self.raise(
                    node.start as usize,
                    "'new.target' can only be used in functions and class static block",
                );
            }
            return Ok(self.finish_node(node, Ty::Other));
        }
        let start_pos = self.start;
        let atom = self.parse_expr_atom(None, ForInit::No, true)?;
        let callee = self.parse_subscripts(atom, start_pos, true, ForInit::No)?;
        if callee.is(Ty::Super) {
            return self.raise(start_pos, "Invalid use of 'super'");
        }
        if self.eat(Tok::ParenL)? {
            self.parse_expr_list(Tok::ParenR, true, false, None)?;
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    /// One chunk of a template. Returns whether it is the last.
    fn parse_template_element(&mut self, is_tagged: bool) -> PResult<bool> {
        if self.ty == Tok::InvalidTemplate && !is_tagged {
            return self.raise(
                self.start,
                "Bad escape sequence in untagged template literal",
            );
        }
        self.next()?;
        Ok(self.ty == Tok::BackQuote)
    }

    pub(crate) fn parse_template(&mut self, is_tagged: bool) -> PResult<usize> {
        self.next()?;
        let mut substitutions = 0;
        let mut tail = self.parse_template_element(is_tagged)?;
        while !tail {
            if self.ty == Tok::Eof {
                return self.raise(self.pos, "Unterminated template literal");
            }
            self.expect(Tok::DollarBraceL)?;
            if self.ts && self.in_type {
                self.ts_parse_type()?;
            } else {
                self.parse_expression(ForInit::No, None)?;
            }
            substitutions += 1;
            self.expect(Tok::BraceR)?;
            tail = self.parse_template_element(is_tagged)?;
        }
        self.next()?;
        Ok(substitutions)
    }

    fn is_async_prop(&self, prop: &Node) -> bool {
        !prop.computed
            && prop.first().is(Ty::Identifier)
            && prop.first().name == "async"
            && (matches!(
                self.ty,
                Tok::Name | Tok::Num | Tok::String | Tok::BracketL | Tok::Star
            ) || self.ty.keyword().is_some())
            && !self.line_break_between(self.last_tok_end, self.start)
    }

    /// An object literal, or an object pattern where one is known to
    /// stand.
    pub(crate) fn parse_obj(
        &mut self,
        is_pattern: bool,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        let mut node = self.start_node();
        let mut first = true;
        let mut proto = false;
        self.next()?;
        while !self.eat(Tok::BraceR)? {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
                if self.after_trailing_comma(Tok::BraceR, false)? {
                    break;
                }
            }
            let prop = self.parse_property(is_pattern, errors.as_deref_mut())?;
            if !is_pattern {
                self.check_prop_clash(&prop, &mut proto, errors.as_deref_mut())?;
            }
            node.list.push(Some(prop));
        }
        Ok(self.finish_node(
            node,
            if is_pattern {
                Ty::ObjectPattern
            } else {
                Ty::ObjectExpression
            },
        ))
    }

    fn parse_property(
        &mut self,
        is_pattern: bool,
        errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        if self.ts && !is_pattern {
            let mut decorated = false;
            while self.ty == Tok::At {
                self.parse_decorator()?;
                decorated = true;
            }
            let property = self.parse_property_base(is_pattern, errors)?;
            if decorated && property.is(Ty::SpreadElement) {
                return self.raise(
                    property.start as usize,
                    "Decorators can't be used with SpreadElement",
                );
            }
            return Ok(property);
        }
        self.parse_property_base(is_pattern, errors)
    }

    fn parse_property_base(
        &mut self,
        is_pattern: bool,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        let mut prop = self.start_node();
        if self.eat(Tok::Ellipsis)? {
            if is_pattern {
                prop.a = Some(Box::new(self.parse_ident(false)?));
                if self.ty == Tok::Comma {
                    self.raise_comma_after_rest(self.start)?;
                }
                return Ok(self.finish_node(prop, Ty::RestElement));
            }
            prop.a = Some(Box::new(
                self.parse_maybe_assign(ForInit::No, errors.as_deref_mut())?,
            ));
            if self.ty == Tok::Comma
                && let Some(errors) = errors
                && errors.trailing_comma < 0
            {
                errors.trailing_comma = self.start as i64;
            }
            return Ok(self.finish_node(prop, Ty::SpreadElement));
        }
        // Unset (zero) where neither a pattern nor an expression that
        // may turn out to be one is read.
        let start_pos = if is_pattern || errors.is_some() {
            self.start
        } else {
            0
        };
        let mut is_generator = !is_pattern && self.eat(Tok::Star)?;
        let contains_esc = self.contains_esc;
        self.parse_property_name(&mut prop)?;
        let is_async = if !is_pattern && !contains_esc && !is_generator && self.is_async_prop(&prop)
        {
            is_generator = self.eat(Tok::Star)?;
            self.parse_property_name(&mut prop)?;
            true
        } else {
            false
        };
        self.parse_property_value(
            &mut prop,
            is_pattern,
            is_generator,
            is_async,
            start_pos,
            errors,
            contains_esc,
        )?;
        Ok(self.finish_node(prop, Ty::Property))
    }

    fn parse_getter_setter(&mut self, prop: &mut Node) -> PResult<()> {
        if self.ts {
            return self.ts_parse_getter_setter(prop);
        }
        let kind = prop.first().name.clone();
        self.parse_property_name(prop)?;
        let value = self.parse_method(false, false, false)?;
        let expected = if kind == "get" { 0 } else { 1 };
        if value.list.len() != expected {
            return self.raise(
                value.start as usize,
                if kind == "get" {
                    "getter should have no params"
                } else {
                    "setter should have exactly one param"
                },
            );
        }
        if kind == "set"
            && let Some(Some(parameter)) = value.list.first()
            && parameter.is(Ty::RestElement)
        {
            return self.raise(parameter.start as usize, "Setter cannot use rest params");
        }
        prop.b = Some(Box::new(value));
        prop.name = kind;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    fn parse_property_value(
        &mut self,
        prop: &mut Node,
        is_pattern: bool,
        is_generator: bool,
        is_async: bool,
        start_pos: usize,
        errors: Option<&mut DestructuringErrors>,
        contains_esc: bool,
    ) -> PResult<()> {
        if self.ts && self.ts_match_left_relational() {
            // A method with type parameters.
            if is_pattern {
                return self.unexpected();
            }
            prop.name = "init".to_string();
            prop.method = true;
            self.ts_try_parse_type_parameters(crate::ts_types::TypeParameterModifiers::Const)?;
            prop.b = Some(Box::new(self.parse_method(
                is_generator,
                is_async,
                false,
            )?));
            return Ok(());
        }
        if (is_generator || is_async) && self.ty == Tok::Colon {
            return self.unexpected();
        }
        let key_is_name = !prop.computed && prop.first().is(Ty::Identifier);

        if self.eat(Tok::Colon)? {
            let value = if is_pattern {
                self.parse_maybe_default(self.start, None)?
            } else {
                self.parse_maybe_assign(ForInit::No, errors)?
            };
            prop.b = Some(Box::new(value));
            prop.name = "init".to_string();
        } else if self.ty == Tok::ParenL {
            if is_pattern {
                return self.unexpected();
            }
            prop.method = true;
            prop.b = Some(Box::new(self.parse_method(
                is_generator,
                is_async,
                false,
            )?));
            prop.name = "init".to_string();
        } else if !is_pattern
            && !contains_esc
            && key_is_name
            && (prop.first().name == "get" || prop.first().name == "set")
            && !matches!(self.ty, Tok::Comma | Tok::BraceR | Tok::Eq)
        {
            if is_generator || is_async {
                return self.unexpected();
            }
            self.parse_getter_setter(prop)?;
        } else if key_is_name {
            if is_generator || is_async {
                return self.unexpected();
            }
            let key = prop.first().clone();
            self.check_unreserved(key.start as usize, &key.name)?;
            if key.name == "await" && self.await_ident_pos == 0 {
                self.await_ident_pos = start_pos;
            }
            let value = if is_pattern {
                self.parse_maybe_default(start_pos, Some(key))?
            } else if self.ty == Tok::Eq
                && let Some(errors) = errors
            {
                if errors.shorthand_assign < 0 {
                    errors.shorthand_assign = self.start as i64;
                }
                self.parse_maybe_default(start_pos, Some(key))?
            } else {
                key
            };
            prop.b = Some(Box::new(value));
            prop.name = "init".to_string();
            prop.shorthand = true;
        } else {
            return self.unexpected();
        }
        Ok(())
    }

    /// A property's or class member's name, into `prop.a`.
    pub(crate) fn parse_property_name(&mut self, prop: &mut Node) -> PResult<()> {
        let key = if self.eat(Tok::BracketL)? {
            prop.computed = true;
            let key = self.parse_maybe_assign(ForInit::No, None)?;
            self.expect(Tok::BracketR)?;
            key
        } else {
            prop.computed = false;
            if self.ty == Tok::Num || self.ty == Tok::String {
                self.parse_expr_atom(None, ForInit::No, false)?
            } else {
                self.parse_ident(true)?
            }
        };
        prop.a = Some(Box::new(key));
        Ok(())
    }

    /// An object or class method, from its parameter list on.
    pub(crate) fn parse_method(
        &mut self,
        is_generator: bool,
        is_async: bool,
        allow_direct_super: bool,
    ) -> PResult<Node> {
        if self.ts {
            return self.ts_parse_method(is_generator, is_async, allow_direct_super, false, None);
        }
        let mut node = self.start_node();
        let old_yield_pos = self.yield_pos;
        let old_await_pos = self.await_pos;
        let old_await_ident_pos = self.await_ident_pos;

        self.yield_pos = 0;
        self.await_pos = 0;
        self.await_ident_pos = 0;
        self.enter_scope(
            function_flags(is_async, is_generator)
                | SCOPE_SUPER
                | if allow_direct_super {
                    SCOPE_DIRECT_SUPER
                } else {
                    0
                },
        );

        self.expect(Tok::ParenL)?;
        node.list = self.parse_binding_list(Tok::ParenR, false, true, None)?;
        self.check_yield_await_in_default_params()?;
        self.parse_function_body(&mut node, false, true, ForInit::No, BodyOf::Other)?;

        self.yield_pos = old_yield_pos;
        self.await_pos = old_await_pos;
        self.await_ident_pos = old_await_ident_pos;
        Ok(self.finish_node(node, Ty::FunctionExpression))
    }

    /// An arrow function, from after its `=>`. `params` is what was
    /// read before it, as expressions.
    pub(crate) fn parse_arrow_expression(
        &mut self,
        mut node: Node,
        mut params: Vec<Option<Node>>,
        is_async: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        if self.ts && self.ty == Tok::Colon {
            self.ts_parse_type_annotation(true)?;
        }
        let old_yield_pos = self.yield_pos;
        let old_await_pos = self.await_pos;
        let old_await_ident_pos = self.await_ident_pos;
        let old_maybe_in_arrow_parameters = self.maybe_in_arrow_parameters;

        self.enter_scope(function_flags(is_async, false) | SCOPE_ARROW);
        self.yield_pos = 0;
        self.await_pos = 0;
        self.await_ident_pos = 0;

        self.maybe_in_arrow_parameters = true;
        self.to_assignable_list(&mut params, true)?;
        self.maybe_in_arrow_parameters = false;
        node.list = params;
        self.parse_function_body(&mut node, true, false, for_init, BodyOf::Other)?;

        self.yield_pos = old_yield_pos;
        self.await_pos = old_await_pos;
        self.await_ident_pos = old_await_ident_pos;
        self.maybe_in_arrow_parameters = old_maybe_in_arrow_parameters;
        Ok(self.finish_node(node, Ty::ArrowFunctionExpression))
    }

    /// A function's body, and the checks on its parameters that wait
    /// for it (a `"use strict"` directive changes them).
    pub(crate) fn parse_function_body(
        &mut self,
        node: &mut Node,
        is_arrow: bool,
        is_method: bool,
        for_init: ForInit,
        body_of: BodyOf,
    ) -> PResult<()> {
        if self.ts && !self.ts_before_function_body(node, body_of)? {
            return Ok(());
        }
        node.has_body = true;
        let is_expression = is_arrow && self.ty != Tok::BraceL;
        let old_strict = self.strict;
        let mut use_strict = false;

        if is_expression {
            self.parse_maybe_assign(for_init, None)?;
            self.check_params(node, false)?;
        } else {
            // acorn-typescript looks through a parameter property
            // (`constructor(public x)`) to the parameter it holds.
            let simple = node.list.iter().flatten().all(|p| {
                if p.is(Ty::TsParameterProperty) {
                    p.first().is(Ty::Identifier)
                } else {
                    p.is(Ty::Identifier)
                }
            });
            if !old_strict || !simple {
                use_strict = self.strict_directive(self.end);
                // acorn-typescript lets this one pass everywhere.
                if use_strict && !simple && !self.ts {
                    return self.raise(
                        node.start as usize,
                        "Illegal 'use strict' directive in function with non-simple parameter list",
                    );
                }
            }
            let old_labels = std::mem::take(&mut self.labels);
            if use_strict {
                self.strict = true;
            }
            self.check_params(
                node,
                !old_strict && !use_strict && !is_arrow && !is_method && simple,
            )?;
            if self.strict
                && let Some(id) = node.a.as_deref()
            {
                let id = id.clone();
                self.check_lval_simple(&id, Bind::Outside, None)?;
            }
            self.parse_block(false, None, use_strict && !old_strict)?;
            self.labels = old_labels;
        }
        self.exit_scope();
        Ok(())
    }

    fn check_params(&mut self, node: &Node, allow_duplicates: bool) -> PResult<()> {
        let mut names = Vec::new();
        for parameter in node.list.iter().flatten() {
            let clashes = if allow_duplicates {
                None
            } else {
                Some(&mut names)
            };
            self.check_lval_inner_pattern(parameter, Bind::Var, clashes)?;
        }
        Ok(())
    }

    /// A comma-separated list of expressions up to `close`.
    pub(crate) fn parse_expr_list(
        &mut self,
        close: Tok,
        allow_trailing_comma: bool,
        allow_empty: bool,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Vec<Option<Node>>> {
        let mut elements = Vec::new();
        let mut first = true;
        while !self.eat(close)? {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
                if allow_trailing_comma && self.after_trailing_comma(close, false)? {
                    break;
                }
            }
            let element = if allow_empty && self.ty == Tok::Comma {
                None
            } else if self.ty == Tok::Ellipsis {
                let spread = self.parse_spread(errors.as_deref_mut())?;
                if self.ts && self.maybe_in_arrow_parameters && self.ty == Tok::Colon {
                    self.ts_parse_type_annotation(true)?;
                }
                if self.ty == Tok::Comma
                    && let Some(errors) = errors.as_deref_mut()
                    && errors.trailing_comma < 0
                {
                    errors.trailing_comma = self.start as i64;
                }
                Some(spread)
            } else {
                // acorn-typescript reads every item as a possible
                // arrow function parameter.
                let paren_item = self.ts;
                Some(self.parse_maybe_assign_item(
                    ForInit::No,
                    errors.as_deref_mut(),
                    paren_item,
                )?)
            };
            elements.push(element);
        }
        Ok(elements)
    }

    /// Refuses a name that is a keyword or reserved where it stands.
    pub(crate) fn check_unreserved(&self, start: usize, name: &str) -> PResult<()> {
        if self.in_generator() && name == "yield" {
            return self.raise(start, "Cannot use 'yield' as identifier inside a generator");
        }
        let lenient = self.ambient_allows_names();
        if !lenient && self.in_async() && name == "await" {
            return self.raise(
                start,
                "Cannot use 'await' as identifier inside an async function",
            );
        }
        if !lenient && self.current_this_scope().flags & SCOPE_VAR == 0 && name == "arguments" {
            return self.raise(start, "Cannot use 'arguments' in class field initializer");
        }
        if !lenient && self.in_class_static_block() && (name == "arguments" || name == "await") {
            return self.raise(
                start,
                format!("Cannot use {name} in class static initialization block"),
            );
        }
        if is_keyword(name) {
            return self.raise(start, format!("Unexpected keyword '{name}'"));
        }
        let reserved = if self.strict {
            is_reserved_strict(name)
        } else {
            is_reserved(name)
        };
        if reserved && !lenient {
            if !self.in_async() && name == "await" {
                return self.raise(
                    start,
                    "Cannot use keyword 'await' outside an async function",
                );
            }
            return self.raise(start, format!("The keyword '{name}' is reserved"));
        }
        Ok(())
    }

    /// A name. `liberal` takes any word: a property name after a dot,
    /// an export's name.
    pub(crate) fn parse_ident(&mut self, liberal: bool) -> PResult<Node> {
        let node = self.parse_ident_node()?;
        self.next_liberal(liberal)?;
        let node = self.finish_node(node, Ty::Identifier);
        if !liberal {
            self.check_unreserved(node.start as usize, &node.name)?;
            if node.name == "await" && self.await_ident_pos == 0 {
                self.await_ident_pos = node.start as usize;
            }
        }
        Ok(node)
    }

    fn parse_ident_node(&mut self) -> PResult<Node> {
        let mut node = self.start_node();
        // acorn-typescript takes any word's text as the name and
        // leaves the token's type alone, except for the `class` and
        // `function` that acorn has a context to close for.
        let closes_context = matches!(self.ty.keyword(), Some("class" | "function"))
            && (self.last_tok_end != self.last_tok_start + 1
                || self.code_at(self.last_tok_start) != 46);
        if self.ts && self.ty.is_keyword_or_identifier() && !closes_context {
            node.name = self.value.clone();
            return Ok(node);
        }
        if self.ty == Tok::Name {
            node.name = self.value.clone();
        } else if let Some(keyword) = self.ty.keyword() {
            node.name = keyword.to_string();
            // `class` and `function` as names leave the context the
            // tokenizer opened for them, unless they follow a dot,
            // where it opened none.
            if (keyword == "class" || keyword == "function")
                && (self.last_tok_end != self.last_tok_start + 1
                    || self.code_at(self.last_tok_start) != 46)
            {
                self.context.pop();
            }
            self.ty = Tok::Name;
        } else {
            return self.unexpected();
        }
        Ok(node)
    }

    pub(crate) fn parse_private_ident(&mut self) -> PResult<Node> {
        let mut node = self.start_node();
        if self.ty == Tok::PrivateId {
            node.name = self.value.clone();
        } else {
            return self.unexpected();
        }
        self.next()?;
        let node = self.finish_node(node, Ty::PrivateIdentifier);
        match self.private_name_stack.last_mut() {
            Some(names) => names.used.push((node.name.clone(), node.start)),
            None => {
                return self.raise(
                    node.start as usize,
                    format!(
                        "Private field '#{}' must be declared in an enclosing class",
                        node.name
                    ),
                );
            }
        }
        Ok(node)
    }

    pub(crate) fn parse_yield(&mut self, for_init: ForInit) -> PResult<Node> {
        if self.yield_pos == 0 {
            self.yield_pos = self.start;
        }
        let node = self.start_node();
        self.next()?;
        let bare = self.ty == Tok::Semi
            || self.can_insert_semicolon()
            || (self.ty != Tok::Star
                && !self.ty.starts_expr()
                && !(self.ts && self.ts_match_left_relational()));
        if !bare {
            self.eat(Tok::Star)?;
            self.parse_maybe_assign(for_init, None)?;
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    fn parse_await(&mut self, for_init: ForInit) -> PResult<Node> {
        if self.await_pos == 0 {
            self.await_pos = self.start;
        }
        let node = self.start_node();
        self.next()?;
        self.parse_maybe_unary(None, true, false, for_init)?;
        Ok(self.finish_node(node, Ty::Other))
    }
}
