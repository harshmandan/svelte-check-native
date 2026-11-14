//! acorn-typescript's versions of acorn's expression methods.

use crate::state::{
    Bind, DestructuringErrors, ForInit, Node, PResult, Parser, SCOPE_DIRECT_SUPER, SCOPE_SUPER, Ty,
    function_flags,
};
use crate::token::Tok;
use crate::ts::ModifierRules;
use crate::ts_types::TypeParameterModifiers;

/// What a function's body belongs to, for acorn-typescript: a function
/// declaration or a class method may go without one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BodyOf {
    Other,
    FunctionDeclaration,
    ClassMethod,
}

/// Where acorn-typescript's part of `parseSubscript` leaves off.
pub(crate) enum SubscriptStart {
    /// It read the subscript itself.
    Done(Node),
    /// acorn's part goes on with this base.
    Continue { base: Node, optional_chained: bool },
}

impl Parser {
    // --- assignment targets ---

    /// `(a: T)` as the parameter it is in an arrow function's list:
    /// the expression, carrying the annotation.
    pub(crate) fn type_cast_to_parameter(cast: Node) -> Node {
        let end = cast.end;
        let annotation = cast.annotation;
        let mut expression = match cast.a {
            Some(expression) => *expression,
            None => return Node::at(cast.start),
        };
        expression.annotation = annotation;
        expression.end = end;
        expression
    }

    /// acorn-typescript's `toAssignable`. Returns the node it hands
    /// back where that is another one than `node`.
    pub(crate) fn ts_to_assignable(
        &mut self,
        node: &mut Node,
        is_binding: bool,
        errors: Option<&DestructuringErrors>,
        preserve_wrapper: bool,
    ) -> PResult<Option<Node>> {
        let wraps = |node: &Node| node.is_ts_wrapper() || node.is(Ty::ParenthesizedExpression);
        match node.ty {
            Ty::ParenthesizedExpression if wraps(node.first()) => {
                self.ts_to_assignable_inner(node, is_binding, errors, preserve_wrapper)
            }
            Ty::ParenthesizedExpression => {
                self.to_assignable_base(node, is_binding, errors)?;
                Ok(None)
            }
            Ty::TsAsExpression
            | Ty::TsSatisfiesExpression
            | Ty::TsNonNullExpression
            | Ty::TsTypeAssertion => {
                if !is_binding {
                    return self.raise(
                        node.start as usize,
                        "Unexpected type cast in parameter position.",
                    );
                }
                self.ts_to_assignable_inner(node, is_binding, errors, preserve_wrapper)
            }
            Ty::MemberExpression => Ok(None),
            Ty::AssignmentExpression => {
                if !is_binding
                    && node.first().is(Ty::TsTypeCastExpression)
                    && let Some(left) = node.a.take()
                {
                    node.a = Some(Box::new(Self::type_cast_to_parameter(*left)));
                }
                self.to_assignable_base(node, is_binding, errors)?;
                Ok(None)
            }
            Ty::TsTypeCastExpression => Ok(Some(Self::type_cast_to_parameter(node.clone()))),
            _ => {
                self.to_assignable_base(node, is_binding, errors)?;
                Ok(None)
            }
        }
    }

    /// Converts what a wrapper wraps, and hands back either the wrapper
    /// (kept around it) or the converted expression.
    fn ts_to_assignable_inner(
        &mut self,
        node: &mut Node,
        is_binding: bool,
        errors: Option<&DestructuringErrors>,
        preserve_wrapper: bool,
    ) -> PResult<Option<Node>> {
        let Some(inner) = node.a.as_deref_mut() else {
            return Ok(None);
        };
        let replaced = self.ts_to_assignable(inner, is_binding, errors, preserve_wrapper)?;
        if preserve_wrapper {
            if let Some(replaced) = replaced {
                node.a = Some(Box::new(replaced));
            }
            return Ok(None);
        }
        Ok(Some(replaced.unwrap_or_else(|| node.first().clone())))
    }

    /// `toAssignable` where the caller takes the node handed back.
    // Named after acorn's `toAssignable`, which converts in place.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_assignable_take(
        &mut self,
        node: &mut Node,
        is_binding: bool,
        errors: Option<&DestructuringErrors>,
        preserve_wrapper: bool,
    ) -> PResult<()> {
        if let Some(replaced) = self.ts_to_assignable(node, is_binding, errors, preserve_wrapper)? {
            *node = replaced;
        }
        Ok(())
    }

    fn is_assignable(node: &Node, is_binding: bool) -> bool {
        match node.ty {
            Ty::TsTypeCastExpression | Ty::ParenthesizedExpression => {
                Self::is_assignable(node.first(), is_binding)
            }
            Ty::TsParameterProperty
            | Ty::Identifier
            | Ty::ObjectPattern
            | Ty::ArrayPattern
            | Ty::AssignmentPattern
            | Ty::RestElement => true,
            Ty::ObjectExpression => {
                let last = node.list.len().saturating_sub(1);
                node.list.iter().flatten().enumerate().all(|(i, prop)| {
                    (i == last || !prop.is(Ty::SpreadElement)) && Self::is_assignable(prop, false)
                })
            }
            Ty::Property => Self::is_assignable(node.second(), false),
            Ty::SpreadElement => Self::is_assignable(node.first(), false),
            Ty::ArrayExpression => node.list.iter().all(|element| {
                element
                    .as_ref()
                    .is_none_or(|e| Self::is_assignable(e, false))
            }),
            Ty::AssignmentExpression => node.name == "=",
            Ty::MemberExpression => !is_binding,
            _ => false,
        }
    }

    /// A parameter of a method or function: decorators, the modifiers
    /// of a parameter property, the pattern, `?`, a type, a default.
    pub(crate) fn ts_parse_assignable_list_item(
        &mut self,
        allow_modifiers: Option<bool>,
    ) -> PResult<Node> {
        while self.ty == Tok::At {
            self.parse_decorator()?;
        }
        let start_pos = self.start;
        let mut is_property = false;
        if let Some(allowed) = allow_modifiers {
            let mut modified = Node::at(start_pos as u32);
            self.ts_parse_modifiers(
                &mut modified,
                &ModifierRules {
                    allowed: &["public", "private", "protected", "override", "readonly"],
                    disallowed: &[],
                    stop_on_start_of_class_static_block: false,
                    error: crate::ts::INVALID_MODIFIER_ON_TYPE_MEMBER,
                },
            )?;
            is_property = modified.accessibility.is_some()
                || modified.has_modifier("readonly")
                || modified.has_modifier("override");
            if !allowed && is_property {
                return self.raise_at_column_of(
                    start_pos,
                    "A parameter property is only allowed in a constructor implementation.",
                );
            }
        }
        let mut left = self.parse_maybe_default(start_pos, None)?;
        self.parse_binding_list_item(&mut left)?;
        let left_start = left.start as usize;
        let element = self.parse_maybe_default(left_start, Some(left))?;
        if is_property {
            let mut property = self.start_node_at(start_pos);
            if !element.is(Ty::Identifier) && !element.is(Ty::AssignmentPattern) {
                return self.raise(
                    property.start as usize,
                    "A parameter property may not be declared using a binding pattern.",
                );
            }
            property.a = Some(Box::new(element));
            return Ok(self.finish_node(property, Ty::TsParameterProperty));
        }
        Ok(element)
    }

    /// What may follow an item between parentheses that turn out to be
    /// an arrow function's parameters: `?`, and `: Type`.
    pub(crate) fn ts_parse_paren_item(&mut self, mut node: Node) -> PResult<Node> {
        let start_pos = self.start;
        if self.eat(Tok::Question)? {
            node.optional = true;
            node.end = self.last_tok_end as u32;
        }
        if self.ty == Tok::Colon {
            let mut cast = self.start_node_at(start_pos);
            let annotation = self.ts_parse_type_annotation(true)?;
            cast.annotation = Some(annotation.start);
            cast.a = Some(Box::new(node));
            return Ok(self.finish_node(cast, Ty::TsTypeCastExpression));
        }
        Ok(node)
    }

    // --- expressions ---

    /// acorn-typescript's `parseMaybeAssign`. Where the expression
    /// starts with `<` it tries an arrow function with type parameters
    /// first and the expression as written second.
    pub(crate) fn ts_parse_maybe_assign(
        &mut self,
        for_init: ForInit,
        mut errors: Option<&mut DestructuringErrors>,
        paren_item: bool,
    ) -> PResult<Node> {
        if !self.ts_match_left_relational() {
            return self.ts_parse_maybe_assign_origin(for_init, errors, paren_item);
        }

        let arrow = self.try_parse(|p| {
            let type_parameters = p.ts_parse_type_parameters(TypeParameterModifiers::Const)?;
            let mut expr =
                p.ts_parse_maybe_assign_origin(for_init, errors.as_deref_mut(), paren_item)?;
            if !expr.is(Ty::ArrowFunctionExpression) {
                return Ok(None);
            }
            expr.start = type_parameters.start;
            Ok(Some(expr))
        })?;
        if let Some(node) = arrow.node {
            return Ok(node);
        }

        let type_cast = self.try_parse(|p| {
            p.ts_parse_maybe_assign_origin(for_init, errors, paren_item)
                .map(Some)
        })?;
        if let Some(node) = type_cast.node {
            return Ok(node);
        }
        // Both failed: the arrow's error is the one thrown.
        if let Some(error) = arrow.error {
            return self.rethrow(error);
        }
        if let Some(error) = type_cast.error {
            return self.rethrow(error);
        }
        self.unexpected()
    }

    fn ts_parse_maybe_assign_origin(
        &mut self,
        for_init: ForInit,
        errors: Option<&mut DestructuringErrors>,
        paren_item: bool,
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
        if self.ty == Tok::ParenL || self.ty.is_identifier() {
            self.potential_arrow_at = start_pos as i64;
            self.potential_arrow_in_for_await = for_init == ForInit::Await;
        }
        let mut left = self.parse_maybe_conditional(for_init, Some(&mut *errors))?;
        if paren_item {
            left = self.ts_parse_paren_item(left)?;
        }
        if self.ty.is_assign() {
            let mut node = self.start_node_at(start_pos);
            node.name = self.value.clone();
            let is_eq = self.ty == Tok::Eq;
            if is_eq {
                self.to_assignable_take(&mut left, true, Some(&*errors), true)?;
            }
            if !own {
                errors.parenthesized_assign = -1;
                errors.trailing_comma = -1;
                errors.double_proto = -1;
            }
            if errors.shorthand_assign >= i64::from(left.start) {
                errors.shorthand_assign = -1;
            }
            if !self.maybe_in_arrow_parameters {
                if is_eq {
                    self.check_lval_pattern(&left, Bind::None, None)?;
                } else {
                    self.check_lval_simple(&left, Bind::None, None)?;
                }
            }
            node.a = Some(Box::new(left));
            self.next()?;
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

    fn ts_parse_conditional(
        &mut self,
        expr: Node,
        start_pos: usize,
        for_init: ForInit,
    ) -> PResult<Node> {
        if self.eat(Tok::Question)? {
            let node = self.start_node_at(start_pos);
            self.parse_maybe_assign(ForInit::No, None)?;
            self.expect(Tok::Colon)?;
            self.parse_maybe_assign(for_init, None)?;
            return Ok(self.finish_node(node, Ty::Other));
        }
        Ok(expr)
    }

    /// acorn-typescript's `parseMaybeConditional`. Among what may be an
    /// arrow function's parameters a `?` may mark an optional one: the
    /// conditional is tried, and dropped where it does not read.
    pub(crate) fn ts_parse_maybe_conditional(
        &mut self,
        for_init: ForInit,
        mut errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        let start_pos = self.start;
        let expr = self.parse_expr_ops(for_init, errors.as_deref_mut())?;
        if self.has_expression_errors(errors.as_deref()) {
            return Ok(expr);
        }
        if !self.maybe_in_arrow_parameters || self.ty != Tok::Question {
            return self.ts_parse_conditional(expr, start_pos, for_init);
        }
        let kept = expr.clone();
        let result =
            self.try_parse(|p| p.ts_parse_conditional(expr, start_pos, for_init).map(Some))?;
        Ok(result.node.unwrap_or(kept))
    }

    /// `expr as T` / `expr satisfies T`, where the parser stands on one
    /// and it binds here. Hands `left` back otherwise.
    pub(crate) fn ts_parse_as_or_satisfies(
        &mut self,
        left: Node,
        left_start: usize,
        min_prec: i32,
    ) -> PResult<Result<Node, Node>> {
        // As tight as `in`.
        if 7 <= min_prec || self.has_preceding_line_break() {
            return Ok(Err(left));
        }
        let satisfies = self.is_contextual("satisfies");
        if !satisfies && !self.is_contextual("as") {
            return Ok(Err(left));
        }
        let mut node = self.start_node_at(left_start);
        node.a = Some(Box::new(left));
        if self.ts_try_next_parse_constant_context()?.is_none() {
            self.ts_next_then_parse_type()?;
        }
        let node = self.finish_node(
            node,
            if satisfies {
                Ty::TsSatisfiesExpression
            } else {
                Ty::TsAsExpression
            },
        );
        self.re_scan_lt_gt();
        Ok(Ok(node))
    }

    /// `<T>expr`. Where that does not read, the `<…>` is read as type
    /// parameters and handed back as if it were the expression.
    pub(crate) fn ts_parse_type_assertion(&mut self) -> PResult<Node> {
        let result = self.try_parse(|p| {
            let mut node = p.start_node();
            if p.ts_try_next_parse_constant_context()?.is_none() {
                p.ts_next_then_parse_type()?;
            }
            p.expect(Tok::Relational)?;
            let expression = p.parse_maybe_unary(None, false, false, ForInit::No)?;
            node.a = Some(Box::new(expression));
            Ok(Some(p.finish_node(node, Ty::TsTypeAssertion)))
        })?;
        match result.node {
            Some(node) if !result.thrown() => Ok(node),
            _ => self.ts_parse_type_parameters(TypeParameterModifiers::Const),
        }
    }

    fn at_possible_async_arrow(&self, base: &Node) -> bool {
        base.is(Ty::Identifier)
            && base.name == "async"
            && self.last_tok_end == base.end as usize
            && !self.can_insert_semicolon()
            && base.end - base.start == 5
            && i64::from(base.start) == self.potential_arrow_at
    }

    /// `async <T>(a) => …`, after the `async`.
    fn ts_try_parse_generic_async_arrow_function(
        &mut self,
        start_pos: usize,
        for_init: ForInit,
    ) -> PResult<Option<Node>> {
        if !self.ts_match_left_relational() {
            return Ok(None);
        }
        let old = self.maybe_in_arrow_parameters;
        self.maybe_in_arrow_parameters = true;
        let head = self.ts_try_parse_and_catch(|p| {
            let node = p.start_node_at(start_pos);
            p.ts_parse_type_parameters(TypeParameterModifiers::Const)?;
            p.expect(Tok::ParenL)?;
            let params = p.parse_binding_list(Tok::ParenR, false, true, None)?;
            p.check_yield_await_in_default_params()?;
            p.ts_try_parse_type_or_type_predicate_annotation()?;
            p.expect(Tok::Arrow)?;
            Ok(Some((node, params)))
        })?;
        self.maybe_in_arrow_parameters = old;
        let Some((node, params)) = head else {
            return Ok(None);
        };
        self.parse_arrow_expression(node, params, true, for_init)
            .map(Some)
    }

    /// What acorn-typescript's `parseSubscript` does before acorn's:
    /// `!`, and type arguments with what may follow them.
    pub(crate) fn ts_parse_subscript_start(
        &mut self,
        mut base: Node,
        start_pos: usize,
        no_calls: bool,
        optional_chained: bool,
        for_init: ForInit,
    ) -> PResult<SubscriptStart> {
        let mut optional_chained = optional_chained;
        if !self.has_preceding_line_break() && self.value == "!" && self.ty == Tok::Prefix {
            self.expr_allowed = false;
            self.next()?;
            let mut non_null = self.start_node_at(start_pos);
            non_null.a = Some(Box::new(base));
            return Ok(SubscriptStart::Done(
                self.finish_node(non_null, Ty::TsNonNullExpression),
            ));
        }

        let mut is_optional_call = false;
        if self.ty == Tok::QuestionDot && self.lookahead_char_code() == 60 {
            if no_calls {
                return Ok(SubscriptStart::Done(base));
            }
            base.optional = true;
            optional_chained = true;
            is_optional_call = true;
            self.next()?;
        }

        if self.ts_match_left_relational() || self.ty == Tok::BitShift {
            let mut missing_paren_at = false;
            let kept = base.clone();
            let result = self.ts_try_parse_and_catch(|p| {
                if !no_calls
                    && p.at_possible_async_arrow(&base)
                    && let Some(arrow) =
                        p.ts_try_parse_generic_async_arrow_function(start_pos, for_init)?
                {
                    return Ok(Some(arrow));
                }
                if !p.ts_parse_type_arguments_in_expression()? {
                    return Ok(Some(base));
                }
                if is_optional_call && p.ty != Tok::ParenL {
                    missing_paren_at = true;
                    return Ok(Some(base));
                }
                if p.ty == Tok::InvalidTemplate || p.ty == Tok::BackQuote {
                    let node = p.start_node_at(start_pos);
                    p.parse_template(true)?;
                    if optional_chained {
                        return p.raise(
                            start_pos,
                            "Tagged Template Literals are not allowed in optionalChain.",
                        );
                    }
                    return Ok(Some(p.finish_node(node, Ty::Other)));
                }
                if !no_calls && p.eat(Tok::ParenL)? {
                    let mut errors = DestructuringErrors::new();
                    let mut node = p.start_node_at(start_pos);
                    let arguments =
                        p.parse_expr_list(Tok::ParenR, true, false, Some(&mut errors))?;
                    for argument in arguments.iter().flatten() {
                        if argument.is(Ty::TsTypeCastExpression) {
                            return p.raise(
                                argument.annotation.unwrap_or(argument.start) as usize,
                                "Did not expect a type annotation here.",
                            );
                        }
                    }
                    node.optional = is_optional_call;
                    p.throw_expression_errors(Some(&errors))?;
                    return Ok(Some(p.finish_node(node, Ty::Other)));
                }
                // `a<b>c` compares; `a<b>` followed by something that
                // cannot continue an expression instantiates.
                let ty = p.ty;
                if p.ts_match_right_relational()
                    || ty == Tok::BitShift
                    || (ty != Tok::ParenL && ty.starts_expr() && !p.has_preceding_line_break())
                {
                    return Ok(None);
                }
                let mut node = p.start_node_at(start_pos);
                node.a = Some(Box::new(base));
                Ok(Some(p.finish_node(node, Ty::TsInstantiationExpression)))
            })?;
            if missing_paren_at {
                return self.unexpected_nowhere();
            }
            if let Some(result) = result {
                if result.is(Ty::TsInstantiationExpression)
                    && (self.ty == Tok::Dot
                        || (self.ty == Tok::QuestionDot && self.lookahead_char_code() != 40))
                {
                    return self.raise(
                        self.start,
                        "Invalid property access after an instantiation expression. You can either wrap the instantiation expression in parentheses, or delete the type arguments.",
                    );
                }
                return Ok(SubscriptStart::Done(result));
            }
            base = kept;
        }
        Ok(SubscriptStart::Continue {
            base,
            optional_chained,
        })
    }

    /// Whether the parenthesized list just read is an arrow function's
    /// parameters. A `:` after it may be the return type.
    pub(crate) fn ts_should_parse_arrow(&mut self, list: &[Option<Node>]) -> PResult<bool> {
        let should = if self.ty == Tok::Colon {
            list.iter()
                .flatten()
                .all(|expr| Self::is_assignable(expr, true))
        } else {
            !self.can_insert_semicolon()
        };
        if !should {
            return Ok(false);
        }
        if self.ty == Tok::Colon {
            let result = self.try_parse(|p| {
                let return_type = p.ts_parse_type_or_type_predicate_annotation(Tok::Colon)?;
                if p.can_insert_semicolon() || p.ty != Tok::Arrow {
                    return Ok(None);
                }
                Ok(Some(return_type))
            })?;
            if result.aborted {
                return Ok(false);
            }
        }
        Ok(self.ty == Tok::Arrow)
    }

    /// The same question after `async(…)`. Steps over the `=>`.
    pub(crate) fn ts_should_parse_async_arrow(&mut self) -> PResult<bool> {
        if self.ty == Tok::Colon {
            let result = self.try_parse(|p| {
                let return_type = p.ts_parse_type_or_type_predicate_annotation(Tok::Colon)?;
                if p.can_insert_semicolon() || p.ty != Tok::Arrow {
                    return Ok(None);
                }
                Ok(Some(return_type))
            })?;
            if result.aborted || result.thrown() {
                return Ok(false);
            }
        }
        Ok(!self.can_insert_semicolon() && self.eat(Tok::Arrow)?)
    }

    /// acorn-typescript's `parseNew`.
    pub(crate) fn ts_parse_new(&mut self) -> PResult<Node> {
        if self.contains_esc {
            return self.raise(self.start, "Escape sequence in keyword new");
        }
        let node = self.start_node();
        self.parse_ident(true)?;
        if self.eat(Tok::Dot)? {
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
        let is_import = self.ty == Tok::KwImport;
        let atom = self.parse_expr_atom(None, ForInit::No, false)?;
        let callee = self.parse_subscripts(atom, start_pos, true, ForInit::No)?;
        if is_import && callee.is(Ty::ImportExpression) {
            return self.raise(start_pos, "Cannot use new with import()");
        }
        if self.eat(Tok::ParenL)? {
            self.parse_expr_list(Tok::ParenR, true, false, None)?;
        }
        Ok(self.finish_node(node, Ty::Other))
    }

    /// acorn-typescript's `parseDynamicImport`.
    pub(crate) fn ts_parse_dynamic_import(&mut self, node: Node) -> PResult<Node> {
        self.next()?;
        self.parse_maybe_assign(ForInit::No, None)?;
        if self.eat(Tok::Comma)? {
            self.parse_expression(ForInit::No, None)?;
        }
        if !self.eat(Tok::ParenR)? {
            let error_pos = self.start;
            if self.eat(Tok::Comma)? && self.eat(Tok::ParenR)? {
                return self.raise(error_pos, "Trailing comma is not allowed in import()");
            }
            return self.unexpected_at(error_pos);
        }
        Ok(self.finish_node(node, Ty::ImportExpression))
    }

    /// acorn-typescript's `parseGetterSetter`: type parameters are read
    /// (an accessor has none, but it reads them), and a `this`
    /// parameter does not count.
    pub(crate) fn ts_parse_getter_setter(&mut self, prop: &mut Node) -> PResult<()> {
        let kind = prop.first().name.clone();
        self.parse_property_name(prop)?;
        self.ts_try_parse_type_parameters(TypeParameterModifiers::Const)?;
        let value = self.parse_method(false, false, false)?;
        let first_is_this = matches!(
            value.list.first(),
            Some(Some(first)) if first.is(Ty::Identifier) && first.name == "this"
        );
        let expected = usize::from(kind != "get") + usize::from(first_is_this);
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

    /// acorn-typescript's `parseMethod`. `method` is the class member
    /// the function is the value of.
    pub(crate) fn ts_parse_method(
        &mut self,
        is_generator: bool,
        is_async: bool,
        allow_direct_super: bool,
        in_class_scope: bool,
        method: Option<&Node>,
    ) -> PResult<Node> {
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
        self.ts_try_parse_type_parameters(TypeParameterModifiers::None)?;
        node.list = self.parse_binding_list(Tok::ParenR, false, true, Some(true))?;
        self.check_yield_await_in_default_params()?;
        self.parse_function_body(
            &mut node,
            false,
            true,
            ForInit::No,
            if in_class_scope {
                BodyOf::ClassMethod
            } else {
                BodyOf::Other
            },
        )?;
        self.yield_pos = old_yield_pos;
        self.await_pos = old_await_pos;
        self.await_ident_pos = old_await_ident_pos;

        if let Some(method) = method
            && method.has_modifier("abstract")
            && node.has_body
        {
            let key = method.first();
            let name = if key.is(Ty::Identifier) && !method.computed {
                key.name.clone()
            } else {
                format!("[{}]", self.slice(key.start as usize, key.end as usize))
            };
            return self.raise(
                method.start as usize,
                format!(
                    "Method '{name}' cannot have an implementation because it is marked abstract."
                ),
            );
        }
        Ok(self.finish_node(node, Ty::FunctionExpression))
    }

    /// What acorn-typescript does before a function's body: the return
    /// type, and a declaration or method that has no body. Returns
    /// whether the body is still to be read.
    pub(crate) fn ts_before_function_body(
        &mut self,
        node: &mut Node,
        body_of: BodyOf,
    ) -> PResult<bool> {
        if self.ty == Tok::Colon {
            self.ts_parse_type_or_type_predicate_annotation(Tok::Colon)?;
        }
        let bodiless = match body_of {
            BodyOf::FunctionDeclaration => Some(Ty::TsDeclareFunction),
            BodyOf::ClassMethod => Some(Ty::TsDeclareMethod),
            BodyOf::Other => None,
        };
        if let Some(bodiless) = bodiless
            && self.ty != Tok::BraceL
            && self.is_line_terminator()?
        {
            self.exit_scope();
            let finished = self.finish_node(node.clone(), bodiless);
            *node = finished;
            return Ok(false);
        }
        if bodiless == Some(Ty::TsDeclareFunction) && self.is_ambient_context {
            return self.raise(
                node.start as usize,
                "An implementation cannot be declared in ambient contexts.",
            );
        }
        Ok(true)
    }
}
