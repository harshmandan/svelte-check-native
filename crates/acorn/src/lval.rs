//! Assignment targets and binding patterns (`lval.js`): turning an
//! expression into a pattern once an `=` or `=>` shows it was one, and
//! checking what a pattern binds or assigns to.

use crate::identifier::is_reserved_strict_bind;
use crate::state::{
    Bind, DestructuringErrors, ForInit, Nest, Node, PResult, Parser, Ty, with_stack,
};
use crate::token::Tok;

impl Parser {
    /// Turns an expression into the pattern it would be on the left of
    /// an assignment or in a parameter list.
    // Named as in acorn, which converts the node in place.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_assignable(
        &mut self,
        node: &mut Node,
        is_binding: bool,
        errors: Option<&DestructuringErrors>,
    ) -> PResult<()> {
        // As deep as the expression is nested, which the parser bounds.
        with_stack(|| {
            if self.ts {
                // acorn's own callers drop what acorn-typescript returns:
                // a wrapper it would unwrap stays where it is.
                return self
                    .ts_to_assignable(node, is_binding, errors, false)
                    .map(|_| ());
            }
            self.to_assignable_base(node, is_binding, errors)
        })
    }

    // Named as in acorn, which converts the node in place.
    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_assignable_base(
        &mut self,
        node: &mut Node,
        is_binding: bool,
        errors: Option<&DestructuringErrors>,
    ) -> PResult<()> {
        match node.ty {
            Ty::Identifier => {
                if self.in_async() && node.name == "await" && !self.ambient_allows_names() {
                    return self.raise(
                        node.start as usize,
                        "Cannot use 'await' as identifier inside an async function",
                    );
                }
            }
            Ty::ObjectPattern | Ty::ArrayPattern | Ty::AssignmentPattern | Ty::RestElement => {}
            Ty::ObjectExpression => {
                node.ty = Ty::ObjectPattern;
                self.check_pattern_errors(errors, true)?;
                for prop in node.list.iter_mut().flatten() {
                    self.to_assignable(prop, is_binding, None)?;
                    if prop.is(Ty::RestElement)
                        && matches!(prop.first().ty, Ty::ArrayPattern | Ty::ObjectPattern)
                    {
                        return self.unexpected_at(prop.first().start as usize);
                    }
                }
            }
            Ty::Property => {
                if node.name != "init" {
                    return self.raise(
                        node.first().start as usize,
                        "Object pattern can't contain getter or setter",
                    );
                }
                if let Some(value) = node.b.as_deref_mut() {
                    self.to_assignable(value, is_binding, None)?;
                }
            }
            Ty::ArrayExpression => {
                node.ty = Ty::ArrayPattern;
                self.check_pattern_errors(errors, true)?;
                self.to_assignable_list(&mut node.list, is_binding)?;
            }
            Ty::SpreadElement => {
                node.ty = Ty::RestElement;
                if let Some(argument) = node.a.as_deref_mut() {
                    self.to_assignable(argument, is_binding, None)?;
                }
                if node.first().is(Ty::AssignmentPattern) {
                    return self.raise(
                        node.first().start as usize,
                        "Rest elements cannot have a default value",
                    );
                }
            }
            Ty::AssignmentExpression => {
                if node.name != "=" {
                    return self.raise(
                        node.first().end as usize,
                        "Only '=' operator can be used for specifying default value.",
                    );
                }
                node.ty = Ty::AssignmentPattern;
                if let Some(left) = node.a.as_deref_mut() {
                    self.to_assignable(left, is_binding, None)?;
                }
            }
            Ty::ParenthesizedExpression => {
                if let Some(expression) = node.a.as_deref_mut() {
                    self.to_assignable(expression, is_binding, errors)?;
                }
            }
            Ty::ChainExpression => {
                return self.raise(
                    node.start as usize,
                    "Optional chaining cannot appear in left-hand side",
                );
            }
            Ty::MemberExpression if !is_binding => {}
            _ => return self.raise(node.start as usize, "Assigning to rvalue"),
        }
        Ok(())
    }

    #[allow(clippy::wrong_self_convention)]
    pub(crate) fn to_assignable_list(
        &mut self,
        list: &mut [Option<Node>],
        is_binding: bool,
    ) -> PResult<()> {
        if self.ts {
            // A `(a: T)` item becomes the parameter it annotates.
            for element in list.iter_mut().flatten() {
                if element.is(Ty::TsTypeCastExpression) {
                    *element = Self::type_cast_to_parameter(element.clone());
                }
            }
        }
        for element in list.iter_mut().flatten() {
            self.to_assignable(element, is_binding, None)?;
        }
        Ok(())
    }

    pub(crate) fn parse_spread(
        &mut self,
        errors: Option<&mut DestructuringErrors>,
    ) -> PResult<Node> {
        let mut node = self.start_node();
        self.next()?;
        node.a = Some(Box::new(self.parse_maybe_assign(ForInit::No, errors)?));
        Ok(self.finish_node(node, Ty::SpreadElement))
    }

    pub(crate) fn parse_rest_binding(&mut self) -> PResult<Node> {
        let mut node = self.start_node();
        self.next()?;
        node.a = Some(Box::new(self.parse_binding_atom()?));
        Ok(self.finish_node(node, Ty::RestElement))
    }

    pub(crate) fn parse_binding_atom(&mut self) -> PResult<Node> {
        self.nested(Nest::Pattern, Self::parse_binding_atom_inner)
    }

    fn parse_binding_atom_inner(&mut self) -> PResult<Node> {
        if self.ts && self.ty == Tok::KwThis {
            return self.parse_ident(true);
        }
        match self.ty {
            Tok::BracketL => {
                let mut node = self.start_node();
                self.next()?;
                node.list = self.parse_binding_list(Tok::BracketR, true, true, None)?;
                Ok(self.finish_node(node, Ty::ArrayPattern))
            }
            Tok::BraceL => self.parse_obj(true, None),
            _ => self.parse_ident(false),
        }
    }

    pub(crate) fn parse_binding_list(
        &mut self,
        close: Tok,
        allow_empty: bool,
        allow_trailing_comma: bool,
        allow_modifiers: Option<bool>,
    ) -> PResult<Vec<Option<Node>>> {
        let mut elements = Vec::new();
        let mut first = true;
        while !self.eat(close)? {
            if first {
                first = false;
            } else {
                self.expect(Tok::Comma)?;
            }
            if allow_empty && self.ty == Tok::Comma {
                elements.push(None);
            } else if allow_trailing_comma && self.after_trailing_comma(close, false)? {
                break;
            } else if self.ty == Tok::Ellipsis {
                let mut rest = self.parse_rest_binding()?;
                self.parse_binding_list_item(&mut rest)?;
                elements.push(Some(rest));
                if self.ty == Tok::Comma {
                    self.raise_comma_after_rest(self.start)?;
                }
                self.expect(close)?;
                break;
            } else {
                elements.push(Some(self.parse_assignable_list_item(allow_modifiers)?));
            }
        }
        Ok(elements)
    }

    /// One parameter or pattern element. `allow_modifiers` is set for a
    /// method's parameters, which may be parameter properties.
    fn parse_assignable_list_item(&mut self, allow_modifiers: Option<bool>) -> PResult<Node> {
        if self.ts {
            return self.ts_parse_assignable_list_item(allow_modifiers);
        }
        self.parse_maybe_default(self.start, None)
    }

    /// What may follow a parameter's pattern: in TypeScript `?` and a
    /// type annotation.
    pub(crate) fn parse_binding_list_item(&mut self, param: &mut Node) -> PResult<()> {
        if !self.ts {
            return Ok(());
        }
        if self.eat(Tok::Question)? {
            param.optional = true;
        }
        if let Some(annotation) = self.ts_try_parse_type_annotation()? {
            param.annotation = Some(annotation.start);
        }
        param.end = self.last_tok_end as u32;
        Ok(())
    }

    /// A binding with an optional default. `left` is the binding when
    /// the caller has read it already.
    pub(crate) fn parse_maybe_default(
        &mut self,
        start_pos: usize,
        left: Option<Node>,
    ) -> PResult<Node> {
        let node = self.parse_maybe_default_base(start_pos, left)?;
        if self.ts
            && node.is(Ty::AssignmentPattern)
            && let Some(annotation) = node.annotation
            && node.second().start < annotation
        {
            return self.raise(
                annotation as usize,
                "Type annotations must come before default assignments, e.g. instead of `age = 25: number` use `age: number = 25`.",
            );
        }
        Ok(node)
    }

    pub(crate) fn parse_maybe_default_base(
        &mut self,
        start_pos: usize,
        left: Option<Node>,
    ) -> PResult<Node> {
        let left = match left {
            Some(left) => left,
            None => self.parse_binding_atom()?,
        };
        if !self.eat(Tok::Eq)? {
            return Ok(left);
        }
        let mut node = self.start_node_at(start_pos);
        node.a = Some(Box::new(left));
        node.b = Some(Box::new(self.parse_maybe_assign(ForInit::No, None)?));
        node.name = "=".to_string();
        Ok(self.finish_node(node, Ty::AssignmentPattern))
    }

    /// Checks a simple target: a name or a member expression.
    /// `clashes` collects parameter names where a repeat is an error.
    pub(crate) fn check_lval_simple(
        &mut self,
        expr: &Node,
        binding: Bind,
        mut clashes: Option<&mut Vec<String>>,
    ) -> PResult<()> {
        // acorn-typescript looks through its wrappers to what they wrap.
        let mut expr = expr;
        while self.ts && expr.is_ts_wrapper() {
            expr = expr.first();
        }
        let is_bind = binding != Bind::None;
        match expr.ty {
            Ty::Identifier => {
                if self.strict
                    && is_reserved_strict_bind(&expr.name)
                    && !self.ambient_allows_names()
                {
                    return self.raise(
                        expr.start as usize,
                        format!(
                            "{}{} in strict mode",
                            if is_bind { "Binding " } else { "Assigning to " },
                            expr.name
                        ),
                    );
                }
                if is_bind {
                    if binding == Bind::Lexical && expr.name == "let" {
                        return self.raise(
                            expr.start as usize,
                            "let is disallowed as a lexically bound name",
                        );
                    }
                    if let Some(clashes) = clashes.as_deref_mut() {
                        if clashes.contains(&expr.name) {
                            return self.raise(expr.start as usize, "Argument name clash");
                        }
                        clashes.push(expr.name.clone());
                    }
                    if binding != Bind::Outside {
                        self.declare_name(&expr.name, binding, expr.start)?;
                    }
                }
                Ok(())
            }
            Ty::ChainExpression => self.raise(
                expr.start as usize,
                "Optional chaining cannot appear in left-hand side",
            ),
            Ty::MemberExpression => {
                if is_bind {
                    return self.raise(expr.start as usize, "Binding member expression");
                }
                Ok(())
            }
            Ty::ParenthesizedExpression => {
                if is_bind {
                    return self.raise(expr.start as usize, "Binding parenthesized expression");
                }
                self.check_lval_simple(expr.first(), binding, clashes)
            }
            _ => self.raise(
                expr.start as usize,
                if is_bind {
                    "Binding rvalue"
                } else {
                    "Assigning to rvalue"
                },
            ),
        }
    }

    pub(crate) fn check_lval_pattern(
        &mut self,
        expr: &Node,
        binding: Bind,
        mut clashes: Option<&mut Vec<String>>,
    ) -> PResult<()> {
        match expr.ty {
            // As deep as the pattern is nested, which the parser bounds.
            Ty::ObjectPattern | Ty::ArrayPattern => with_stack(|| {
                for element in expr.list.iter().flatten() {
                    self.check_lval_inner_pattern(element, binding, clashes.as_deref_mut())?;
                }
                Ok(())
            }),
            _ => self.check_lval_simple(expr, binding, clashes),
        }
    }

    pub(crate) fn check_lval_inner_pattern(
        &mut self,
        expr: &Node,
        binding: Bind,
        clashes: Option<&mut Vec<String>>,
    ) -> PResult<()> {
        if expr.is(Ty::TsParameterProperty) {
            return self.check_lval_inner_pattern(expr.first(), binding, clashes);
        }
        match expr.ty {
            Ty::Property => self.check_lval_inner_pattern(expr.second(), binding, clashes),
            Ty::AssignmentPattern | Ty::RestElement => {
                self.check_lval_pattern(expr.first(), binding, clashes)
            }
            _ => self.check_lval_pattern(expr, binding, clashes),
        }
    }
}
