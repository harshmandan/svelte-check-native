//! acorn-typescript's type parser: everything after a `:`, inside
//! `<…>`, and in the body of an interface or type alias.

use crate::state::{ForInit, Node, PResult, Parser, Stop, Ty};
use crate::token::Tok;
use crate::ts::{
    INVALID_MODIFIER_ON_TYPE_MEMBER, INVALID_MODIFIER_ON_TYPE_PARAMETER,
    INVALID_MODIFIER_ON_TYPE_PARAMETER_POSITIONS, ListKind, ModifierRules,
};

/// Which modifiers a type parameter may carry where it stands.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TypeParameterModifiers {
    /// None: a method signature, a function type.
    None,
    /// `const`: a function, a method, an arrow function.
    Const,
    /// `in` and `out`: an interface, a type alias.
    InOut,
    /// `const`, `in` and `out`: a class.
    Class,
}

const NOT_ON_TYPE_PARAMETER: &[&str] = &[
    "public",
    "private",
    "protected",
    "readonly",
    "declare",
    "abstract",
    "override",
];

fn keyword_type(word: &str) -> bool {
    matches!(
        word,
        "any"
            | "boolean"
            | "bigint"
            | "never"
            | "number"
            | "object"
            | "string"
            | "symbol"
            | "undefined"
            | "unknown"
    )
}

/// A signature's parameters and where its return type starts.
pub(crate) struct Signature {
    pub parameters: Vec<Option<Node>>,
    pub return_type_start: Option<u32>,
}

impl Parser {
    // --- annotations ---

    /// `: Type`. With `eat_colon` unset the colon is behind already.
    pub(crate) fn ts_parse_type_annotation(&mut self, eat_colon: bool) -> PResult<Node> {
        let node = self.start_node();
        self.ts_parse_type_annotation_into(eat_colon, node)
    }

    fn ts_parse_type_annotation_into(&mut self, eat_colon: bool, node: Node) -> PResult<Node> {
        self.ts_in_type(|p| {
            if eat_colon {
                p.expect(Tok::Colon)?;
            }
            p.ts_parse_type()
        })?;
        Ok(self.finish_node(node, Ty::TsType))
    }

    pub(crate) fn ts_try_parse_type_annotation(&mut self) -> PResult<Option<Node>> {
        if self.ty == Tok::Colon {
            return self.ts_parse_type_annotation(true).map(Some);
        }
        Ok(None)
    }

    /// The type after `token`, when the parser stands on it.
    fn ts_eat_then_parse_type(&mut self, token: Tok) -> PResult<Option<Node>> {
        if self.ty != token {
            return Ok(None);
        }
        self.ts_next_then_parse_type().map(Some)
    }

    pub(crate) fn ts_next_then_parse_type(&mut self) -> PResult<Node> {
        self.ts_in_type(|p| {
            p.next()?;
            p.ts_parse_type()
        })
    }

    fn ts_expect_then_parse_type(&mut self, token: Tok) -> PResult<Node> {
        self.ts_in_type(|p| {
            p.expect(token)?;
            p.ts_parse_type()
        })
    }

    /// `name is` at the start of a return type.
    fn ts_parse_type_predicate_prefix(&mut self) -> PResult<Option<()>> {
        self.parse_ident(false)?;
        if self.is_contextual("is") && !self.has_preceding_line_break() {
            self.next()?;
            return Ok(Some(()));
        }
        Ok(None)
    }

    fn ts_parse_type_predicate_asserts(&mut self) -> PResult<Option<()>> {
        if self.ty != Tok::TsAsserts {
            return Ok(None);
        }
        let contains_esc = self.contains_esc;
        self.next()?;
        if !self.ty.is_identifier() && self.ty != Tok::KwThis {
            return Ok(None);
        }
        if contains_esc {
            return self.raise(self.last_tok_start, "Escape sequence in keyword asserts");
        }
        Ok(Some(()))
    }

    fn ts_parse_this_type_or_this_type_predicate(&mut self) -> PResult<Node> {
        let node = self.start_node();
        self.next()?;
        let this_type = self.finish_node(node, Ty::TsThisType);
        if self.is_contextual("is") && !self.has_preceding_line_break() {
            self.next()?;
            let predicate = self.start_node_at(this_type.start as usize);
            self.ts_parse_type_annotation(false)?;
            return Ok(self.finish_node(predicate, Ty::TsTypePredicate));
        }
        Ok(this_type)
    }

    /// A return type, which may be a type predicate (`x is T`,
    /// `asserts x`, `asserts x is T`), after `return_token`.
    pub(crate) fn ts_parse_type_or_type_predicate_annotation(
        &mut self,
        return_token: Tok,
    ) -> PResult<Node> {
        self.ts_in_type(|p| {
            let annotation = p.start_node();
            p.expect(return_token)?;
            let has_variable = p.ty.is_identifier()
                && p.ts_try_parse(Self::ts_parse_type_predicate_prefix)?
                    .is_some();
            if has_variable {
                p.ts_parse_type_annotation(false)?;
                return Ok(p.finish_node(annotation, Ty::TsType));
            }
            let asserts = p
                .ts_try_parse(Self::ts_parse_type_predicate_asserts)?
                .is_some();
            if asserts && p.ty == Tok::KwThis {
                p.ts_parse_this_type_or_this_type_predicate()?;
                return Ok(p.finish_node(annotation, Ty::TsType));
            }
            let has_asserted_variable = asserts
                && p.ty.is_identifier()
                && p.ts_try_parse(Self::ts_parse_type_predicate_prefix)?
                    .is_some();
            if !has_asserted_variable {
                if !asserts {
                    return p.ts_parse_type_annotation_into(false, annotation);
                }
                p.parse_ident(false)?;
                return Ok(p.finish_node(annotation, Ty::TsType));
            }
            p.ts_parse_type_annotation(false)?;
            Ok(p.finish_node(annotation, Ty::TsType))
        })
    }

    pub(crate) fn ts_try_parse_type_or_type_predicate_annotation(
        &mut self,
    ) -> PResult<Option<Node>> {
        if self.ty == Tok::Colon {
            return self
                .ts_parse_type_or_type_predicate_annotation(Tok::Colon)
                .map(Some);
        }
        Ok(None)
    }

    // --- type parameters and arguments ---

    fn ts_parse_type_parameter(&mut self, modifiers: TypeParameterModifiers) -> PResult<Node> {
        let mut node = self.start_node();
        let rules = match modifiers {
            TypeParameterModifiers::None => ModifierRules {
                allowed: &[],
                disallowed: &["in", "out"],
                stop_on_start_of_class_static_block: false,
                error: INVALID_MODIFIER_ON_TYPE_PARAMETER_POSITIONS,
            },
            TypeParameterModifiers::Const => ModifierRules {
                allowed: &["const"],
                disallowed: &["in", "out"],
                stop_on_start_of_class_static_block: false,
                error: INVALID_MODIFIER_ON_TYPE_PARAMETER_POSITIONS,
            },
            TypeParameterModifiers::InOut => ModifierRules {
                allowed: &["in", "out"],
                disallowed: NOT_ON_TYPE_PARAMETER,
                stop_on_start_of_class_static_block: false,
                error: INVALID_MODIFIER_ON_TYPE_PARAMETER,
            },
            TypeParameterModifiers::Class => ModifierRules {
                allowed: &["const", "in", "out"],
                disallowed: NOT_ON_TYPE_PARAMETER,
                stop_on_start_of_class_static_block: false,
                error: INVALID_MODIFIER_ON_TYPE_PARAMETER,
            },
        };
        self.ts_parse_modifiers(&mut node, &rules)?;
        node.name = self.parse_ident(false)?.name;
        self.ts_eat_then_parse_type(Tok::KwExtends)?;
        self.ts_eat_then_parse_type(Tok::Eq)?;
        Ok(self.finish_node(node, Ty::TsType))
    }

    /// `<T, U extends V = W>`.
    pub(crate) fn ts_parse_type_parameters(
        &mut self,
        modifiers: TypeParameterModifiers,
    ) -> PResult<Node> {
        let mut node = self.start_node();
        if self.ts_match_left_relational() {
            self.next()?;
        } else {
            return self.unexpected();
        }
        let (parameters, _trailing_comma) = self
            .ts_parse_delimited_list(ListKind::TypeParametersOrArguments, |p| {
                p.ts_parse_type_parameter(modifiers)
            })?;
        self.expect(Tok::Relational)?;
        if parameters.is_empty() {
            return self.raise(self.start, "Type parameter list cannot be empty.");
        }
        node.list = parameters.into_iter().map(Some).collect();
        Ok(self.finish_node(node, Ty::TsTypeParameterDeclaration))
    }

    pub(crate) fn ts_try_parse_type_parameters(
        &mut self,
        modifiers: TypeParameterModifiers,
    ) -> PResult<Option<Node>> {
        if self.ts_match_left_relational() {
            return self.ts_parse_type_parameters(modifiers).map(Some);
        }
        Ok(None)
    }

    /// `<A, B>` after a name. `in_expression` is where a `>` may have
    /// been the start of a longer operator.
    pub(crate) fn ts_parse_type_arguments(&mut self, in_expression: bool) -> PResult<()> {
        let count = self.ts_in_type(|p| {
            p.ts_in_no_context(|p| {
                p.expect(Tok::Relational)?;
                let (types, _) = p
                    .ts_parse_delimited_list(ListKind::TypeParametersOrArguments, |p| {
                        p.ts_parse_type()
                    })?;
                Ok(types.len())
            })
        })?;
        if count == 0 {
            return self.raise(self.start, "Type argument list cannot be empty.");
        }
        if in_expression {
            self.re_scan_lt_gt();
        }
        if !self.ts_match_right_relational() {
            return self.unexpected();
        }
        self.expr_allowed = false;
        self.next()
    }

    /// Type arguments in an expression, where they exist. `false` where
    /// the parser does not stand on a `<`.
    pub(crate) fn ts_parse_type_arguments_in_expression(&mut self) -> PResult<bool> {
        if self.re_scan_lt() != Tok::Relational {
            return Ok(false);
        }
        self.ts_parse_type_arguments(true)?;
        Ok(true)
    }

    // --- names and references ---

    pub(crate) fn ts_parse_entity_name(&mut self, allow_reserved_words: bool) -> PResult<Node> {
        let mut entity = self.parse_ident(allow_reserved_words)?;
        while self.eat(Tok::Dot)? {
            let node = self.start_node_at(entity.start as usize);
            self.parse_ident(allow_reserved_words)?;
            entity = self.finish_node(node, Ty::TsType);
        }
        Ok(entity)
    }

    fn ts_parse_type_reference(&mut self) -> PResult<Node> {
        let mut node = self.start_node();
        let name = self.ts_parse_entity_name(true)?;
        if !self.has_preceding_line_break() && self.ts_match_left_relational() {
            self.ts_parse_type_arguments(false)?;
            node.has_type_arguments = true;
        }
        node.a = Some(Box::new(name));
        Ok(self.finish_node(node, Ty::TsTypeReference))
    }

    /// `const` after `as` or in an assertion, when it is there.
    pub(crate) fn ts_try_next_parse_constant_context(&mut self) -> PResult<Option<Node>> {
        if self.lookahead(1)?.ty != Tok::KwConst {
            return Ok(None);
        }
        self.next()?;
        let reference = self.ts_parse_type_reference()?;
        if reference.has_type_arguments {
            return self.raise(
                reference.first().start as usize,
                "Cannot find name 'const'.",
            );
        }
        Ok(Some(reference))
    }

    fn ts_parse_import_type(&mut self) -> PResult<Node> {
        let node = self.start_node();
        self.expect(Tok::KwImport)?;
        self.expect(Tok::ParenL)?;
        if self.ty != Tok::String {
            return self.raise(
                self.start,
                "Argument in a type import must be a string literal.",
            );
        }
        self.parse_expr_atom(None, ForInit::No, false)?;
        self.expect(Tok::ParenR)?;
        if self.eat(Tok::Dot)? {
            self.ts_parse_entity_name(true)?;
        }
        if self.ts_match_left_relational() {
            self.ts_parse_type_arguments(false)?;
        }
        Ok(self.finish_node(node, Ty::TsType))
    }

    fn ts_parse_type_query(&mut self) -> PResult<Node> {
        let node = self.start_node();
        self.expect(Tok::KwTypeof)?;
        if self.ty == Tok::KwImport {
            self.ts_parse_import_type()?;
        } else {
            self.ts_parse_entity_name(true)?;
        }
        if !self.has_preceding_line_break() && self.ts_match_left_relational() {
            self.ts_parse_type_arguments(false)?;
        }
        Ok(self.finish_node(node, Ty::TsType))
    }

    // --- signatures and members ---

    fn ts_parse_binding_list_for_signature(&mut self) -> PResult<Vec<Option<Node>>> {
        let patterns = self.parse_binding_list(Tok::ParenR, true, true, None)?;
        for pattern in &patterns {
            // A hole between two commas is no pattern, and
            // acorn-typescript asks for its type all the same.
            let Some(pattern) = pattern else {
                return Err(Stop::Crash(
                    "Cannot read properties of null (reading 'type')",
                ));
            };
            if !matches!(
                pattern.ty,
                Ty::Identifier | Ty::RestElement | Ty::ObjectPattern | Ty::ArrayPattern
            ) {
                let name = match pattern.ty {
                    Ty::AssignmentPattern => "AssignmentPattern",
                    Ty::TsParameterProperty => "TSParameterProperty",
                    _ => "Unknown",
                };
                return self.raise(
                    pattern.start as usize,
                    format!(
                        "Name in a signature must be an Identifier, ObjectPattern or ArrayPattern, instead got {name}."
                    ),
                );
            }
        }
        Ok(patterns)
    }

    /// The type parameters, parameters and return type of a signature.
    /// After `=>` the return type has to be there.
    fn ts_fill_signature(&mut self, return_token: Tok) -> PResult<Signature> {
        self.ts_try_parse_type_parameters(TypeParameterModifiers::None)?;
        self.expect(Tok::ParenL)?;
        let parameters = self.ts_parse_binding_list_for_signature()?;
        let return_type_start = if return_token == Tok::Arrow || self.ty == return_token {
            Some(
                self.ts_parse_type_or_type_predicate_annotation(return_token)?
                    .start,
            )
        } else {
            None
        };
        Ok(Signature {
            parameters,
            return_type_start,
        })
    }

    pub(crate) fn ts_parse_type_member_semicolon(&mut self) -> PResult<()> {
        if !self.eat(Tok::Comma)? && !self.is_line_terminator()? {
            self.expect(Tok::Semi)?;
        }
        Ok(())
    }

    fn ts_is_unambiguously_index_signature(&mut self) -> PResult<bool> {
        self.next()?;
        if self.ty.is_identifier() {
            self.next()?;
            return Ok(self.ty == Tok::Colon);
        }
        Ok(false)
    }

    /// `[key: string]: Type`, when that is what stands here.
    pub(crate) fn ts_try_parse_index_signature(&mut self, node: &mut Node) -> PResult<bool> {
        if !(self.ty == Tok::BracketL
            && self.ts_look_ahead(Self::ts_is_unambiguously_index_signature)?)
        {
            return Ok(false);
        }
        self.expect(Tok::BracketL)?;
        self.parse_ident(false)?;
        self.ts_parse_type_annotation(true)?;
        self.expect(Tok::BracketR)?;
        self.ts_try_parse_type_annotation()?;
        self.ts_parse_type_member_semicolon()?;
        let finished = self.finish_node(node.clone(), Ty::TsIndexSignature);
        *node = finished;
        Ok(true)
    }

    fn is_this_param(parameter: &Node) -> bool {
        parameter.is(Ty::Identifier) && parameter.name == "this"
    }

    fn ts_parse_property_or_method_signature(
        &mut self,
        node: Node,
        readonly: bool,
        kind: Option<&str>,
    ) -> PResult<Node> {
        self.eat(Tok::Question)?;
        if self.ty == Tok::ParenL || self.ts_match_left_relational() {
            if readonly {
                return self.raise(
                    node.start as usize,
                    "'readonly' modifier can only appear on a property declaration or index signature.",
                );
            }
            if kind.is_some() && self.ts_match_left_relational() {
                return self.raise(self.start, "An accessor cannot have type parameters.");
            }
            let signature = self.ts_fill_signature(Tok::Colon)?;
            self.ts_parse_type_member_semicolon()?;
            const NO_PARAMETERS: &str = "A 'get' accesor must not have any formal parameters.";
            match kind {
                Some("get") => {
                    if !signature.parameters.is_empty() {
                        return self.raise(self.start, NO_PARAMETERS);
                    }
                }
                Some("set") => {
                    if signature.parameters.len() != 1 {
                        return self.raise(self.start, NO_PARAMETERS);
                    }
                    if let Some(first) = &signature.parameters[0] {
                        if Self::is_this_param(first) {
                            return self.raise(
                                self.start,
                                "'get' and 'set' accessors cannot declare 'this' parameters.",
                            );
                        }
                        if first.is(Ty::Identifier) && first.optional {
                            return self.raise(
                                self.start,
                                "A 'set' accessor cannot have an optional parameter.",
                            );
                        }
                        if first.is(Ty::RestElement) {
                            return self
                                .raise(self.start, "A 'set' accessor cannot have rest parameter.");
                        }
                    }
                    if let Some(start) = signature.return_type_start {
                        return self.raise(
                            start as usize,
                            "A 'set' accessor cannot have a return type annotation.",
                        );
                    }
                }
                _ => {}
            }
            return Ok(self.finish_node(node, Ty::TsType));
        }
        self.ts_try_parse_type_annotation()?;
        self.ts_parse_type_member_semicolon()?;
        Ok(self.finish_node(node, Ty::TsType))
    }

    pub(crate) fn ts_parse_type_member(&mut self) -> PResult<Node> {
        let mut node = self.start_node();
        if self.ty == Tok::ParenL || self.ts_match_left_relational() {
            self.ts_fill_signature(Tok::Colon)?;
            self.ts_parse_type_member_semicolon()?;
            return Ok(self.finish_node(node, Ty::TsType));
        }
        if self.ty == Tok::KwNew {
            self.next()?;
            if self.ty == Tok::ParenL || self.ts_match_left_relational() {
                self.ts_fill_signature(Tok::Colon)?;
                self.ts_parse_type_member_semicolon()?;
                return Ok(self.finish_node(node, Ty::TsType));
            }
            // A property named `new`.
            return self.ts_parse_property_or_method_signature(node, false, None);
        }
        self.ts_parse_modifiers(
            &mut node,
            &ModifierRules {
                allowed: &["readonly"],
                disallowed: &[
                    "declare",
                    "abstract",
                    "private",
                    "protected",
                    "public",
                    "static",
                    "override",
                ],
                stop_on_start_of_class_static_block: false,
                error: INVALID_MODIFIER_ON_TYPE_MEMBER,
            },
        )?;
        if self.ts_try_parse_index_signature(&mut node)? {
            return Ok(node);
        }
        self.parse_property_name(&mut node)?;
        let mut kind = None;
        if !node.computed
            && node.first().is(Ty::Identifier)
            && (node.first().name == "get" || node.first().name == "set")
            && self.ts_can_follow_modifier()
        {
            kind = Some(node.first().name.clone());
            self.parse_property_name(&mut node)?;
        }
        let readonly = node.has_modifier("readonly");
        self.ts_parse_property_or_method_signature(node, readonly, kind.as_deref())
    }

    pub(crate) fn ts_parse_object_type_members(&mut self) -> PResult<()> {
        self.expect(Tok::BraceL)?;
        self.ts_parse_list(ListKind::TypeMembers, Self::ts_parse_type_member)?;
        self.expect(Tok::BraceR)
    }

    /// The types after `extends` or `implements`.
    pub(crate) fn ts_parse_heritage_clause(&mut self, token: &str) -> PResult<()> {
        let original_start = self.start;
        let (list, _) = self.ts_parse_delimited_list(ListKind::HeritageClauseElement, |p| {
            p.ts_parse_entity_name(true)?;
            if p.ts_match_left_relational() {
                p.ts_parse_type_arguments(false)?;
            }
            Ok(())
        })?;
        if list.is_empty() {
            return self.raise(original_start, format!("'{token}' list cannot be empty."));
        }
        Ok(())
    }

    // --- function types ---

    fn ts_skip_parameter_start(&mut self) -> bool {
        if self.ty.is_identifier() || self.ty == Tok::KwThis {
            return self.next().is_ok();
        }
        if self.ty == Tok::BraceL {
            return self.parse_obj(true, None).is_ok();
        }
        if self.ty == Tok::BracketL {
            if self.next().is_err() {
                return false;
            }
            return self
                .parse_binding_list(Tok::BracketR, true, true, None)
                .is_ok();
        }
        false
    }

    fn ts_is_unambiguously_start_of_function_type(&mut self) -> PResult<bool> {
        self.next()?;
        if self.ty == Tok::ParenR || self.ty == Tok::Ellipsis {
            return Ok(true);
        }
        if self.ts_skip_parameter_start() {
            if matches!(self.ty, Tok::Colon | Tok::Comma | Tok::Question | Tok::Eq) {
                return Ok(true);
            }
            if self.ty == Tok::ParenR {
                self.next()?;
                if self.ty == Tok::Arrow {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    fn ts_is_start_of_function_type(&mut self) -> PResult<bool> {
        if self.ts_match_left_relational() {
            return Ok(true);
        }
        Ok(self.ty == Tok::ParenL
            && self.ts_look_ahead(Self::ts_is_unambiguously_start_of_function_type)?)
    }

    /// `(a: A) => R`, or `new (a: A) => R`, `abstract new …`.
    fn ts_parse_function_or_constructor_type(
        &mut self,
        constructor: bool,
        is_abstract: bool,
    ) -> PResult<Node> {
        let node = self.start_node();
        if constructor {
            if is_abstract {
                self.next()?;
            }
            self.next()?;
        }
        self.ts_in_disallow_conditional_types_context(false, |p| p.ts_fill_signature(Tok::Arrow))?;
        Ok(self.finish_node(node, Ty::TsType))
    }

    // --- the type grammar, loosest binding last ---

    fn ts_is_start_of_mapped_type(&mut self) -> PResult<bool> {
        self.next()?;
        if self.eat(Tok::PlusMin)? {
            return Ok(self.ts_is_contextual(Tok::TsReadonly));
        }
        if self.ts_is_contextual(Tok::TsReadonly) {
            self.next()?;
        }
        if self.ty != Tok::BracketL {
            return Ok(false);
        }
        self.next()?;
        if !self.ty.is_identifier() {
            return Ok(false);
        }
        self.next()?;
        Ok(self.ty == Tok::KwIn)
    }

    fn ts_parse_mapped_type(&mut self) -> PResult<Node> {
        let node = self.start_node();
        self.expect(Tok::BraceL)?;
        if self.ty == Tok::PlusMin {
            self.next()?;
            self.expect_contextual("readonly")?;
        } else {
            self.eat_contextual("readonly")?;
        }
        self.expect(Tok::BracketL)?;
        self.parse_ident(false)?;
        self.ts_expect_then_parse_type(Tok::KwIn)?;
        if self.eat_contextual("as")? {
            self.ts_parse_type()?;
        }
        self.expect(Tok::BracketR)?;
        if self.ty == Tok::PlusMin {
            self.next()?;
            self.expect(Tok::Question)?;
        } else {
            self.eat(Tok::Question)?;
        }
        self.ts_eat_then_parse_type(Tok::Colon)?;
        self.semicolon()?;
        self.expect(Tok::BraceR)?;
        Ok(self.finish_node(node, Ty::TsType))
    }

    fn ts_is_tuple_element_label(&mut self) -> PResult<bool> {
        if !self.ty.is_keyword_or_identifier() {
            return Ok(false);
        }
        let next = self.lookahead(1)?;
        if next.ty == Tok::Colon {
            return Ok(true);
        }
        if next.ty != Tok::Question {
            return Ok(false);
        }
        Ok(self.lookahead(2)?.ty == Tok::Colon)
    }

    fn ts_parse_tuple_element_type(&mut self) -> PResult<Node> {
        let start_pos = self.start;
        let rest = self.eat(Tok::Ellipsis)?;
        let mut element;
        if self.ts_is_tuple_element_label()? {
            let mut labeled = self.start_node();
            self.parse_ident(true)?;
            labeled.optional = self.eat(Tok::Question)?;
            self.expect(Tok::Colon)?;
            self.ts_parse_type()?;
            element = self.finish_node(labeled, Ty::TsNamedTupleMember);
        } else {
            element = self.ts_parse_type()?;
            let optional = self.eat(Tok::Question)?;
            if self.eat(Tok::Colon)? {
                return self.raise(
                    element.start as usize,
                    "Tuple members must be labeled with a simple identifier.",
                );
            }
            if optional {
                let wrapper = self.start_node_at(element.start as usize);
                element = self.finish_node(wrapper, Ty::TsOptionalType);
            }
        }
        if rest {
            let wrapper = self.start_node_at(start_pos);
            element = self.finish_node(wrapper, Ty::TsRestType);
        }
        Ok(element)
    }

    fn ts_parse_tuple_type(&mut self) -> PResult<Node> {
        let node = self.start_node();
        self.expect(Tok::BracketL)?;
        let (elements, _) = self.ts_parse_delimited_list(
            ListKind::TupleElementTypes,
            Self::ts_parse_tuple_element_type,
        )?;
        self.expect(Tok::BracketR)?;
        let mut seen_optional = false;
        for element in &elements {
            let optional_member = element.is(Ty::TsNamedTupleMember) && element.optional;
            if seen_optional
                && !element.is(Ty::TsRestType)
                && !element.is(Ty::TsOptionalType)
                && !optional_member
            {
                return self.raise(
                    element.start as usize,
                    "A required element cannot follow an optional element.",
                );
            }
            seen_optional |= optional_member || element.is(Ty::TsOptionalType);
        }
        Ok(self.finish_node(node, Ty::TsTupleType))
    }

    fn ts_parse_non_array_type(&mut self) -> PResult<Node> {
        match self.ty {
            Tok::String | Tok::Num | Tok::KwTrue | Tok::KwFalse => {
                let node = self.start_node();
                self.parse_expr_atom(None, ForInit::No, false)?;
                return Ok(self.finish_node(node, Ty::TsType));
            }
            Tok::PlusMin => {
                if self.value == "-" {
                    let node = self.start_node();
                    if self.lookahead(1)?.ty != Tok::Num {
                        return self.unexpected();
                    }
                    self.parse_maybe_unary(None, false, false, ForInit::No)?;
                    return Ok(self.finish_node(node, Ty::TsType));
                }
            }
            Tok::KwThis => return self.ts_parse_this_type_or_this_type_predicate(),
            Tok::KwTypeof => return self.ts_parse_type_query(),
            Tok::KwImport => return self.ts_parse_import_type(),
            Tok::BraceL => {
                if self.ts_look_ahead(Self::ts_is_start_of_mapped_type)? {
                    return self.ts_parse_mapped_type();
                }
                let node = self.start_node();
                self.ts_parse_object_type_members()?;
                return Ok(self.finish_node(node, Ty::TsType));
            }
            Tok::BracketL => return self.ts_parse_tuple_type(),
            Tok::ParenL => {
                let node = self.start_node();
                self.expect(Tok::ParenL)?;
                self.ts_parse_type()?;
                self.expect(Tok::ParenR)?;
                return Ok(self.finish_node(node, Ty::TsType));
            }
            Tok::BackQuote | Tok::DollarBraceL => {
                let node = self.start_node();
                self.parse_template(false)?;
                return Ok(self.finish_node(node, Ty::TsType));
            }
            ty => {
                if ty.is_keyword_or_identifier() || ty == Tok::KwVoid || ty == Tok::KwNull {
                    let is_keyword_type =
                        ty == Tok::KwVoid || ty == Tok::KwNull || keyword_type(&self.value);
                    if is_keyword_type && self.lookahead_char_code() != 46 {
                        let node = self.start_node();
                        self.next()?;
                        return Ok(self.finish_node(node, Ty::TsType));
                    }
                    return self.ts_parse_type_reference();
                }
            }
        }
        self.unexpected()
    }

    fn ts_parse_array_type_or_higher(&mut self) -> PResult<Node> {
        let mut element = self.ts_parse_non_array_type()?;
        while !self.has_preceding_line_break() && self.eat(Tok::BracketL)? {
            let node = self.start_node_at(element.start as usize);
            if self.ty == Tok::BracketR {
                self.expect(Tok::BracketR)?;
                element = self.finish_node(node, Ty::TsArrayType);
            } else {
                self.ts_parse_type()?;
                self.expect(Tok::BracketR)?;
                element = self.finish_node(node, Ty::TsType);
            }
        }
        Ok(element)
    }

    fn ts_parse_constraint_for_infer_type(&mut self) -> PResult<Option<()>> {
        if self.eat(Tok::KwExtends)? {
            self.ts_in_disallow_conditional_types_context(true, Self::ts_parse_type)?;
            if self.in_disallow_conditional_types_context || self.ty != Tok::Question {
                return Ok(Some(()));
            }
        }
        Ok(None)
    }

    fn ts_parse_type_operator_or_higher(&mut self) -> PResult<Node> {
        let is_type_operator =
            matches!(self.ty, Tok::TsKeyof | Tok::TsReadonly | Tok::TsUnique) && !self.contains_esc;
        if is_type_operator {
            let node = self.start_node();
            let operator = self.value.clone();
            self.next()?;
            let operand = self.ts_parse_type_operator_or_higher()?;
            if operator == "readonly"
                && !operand.is(Ty::TsTupleType)
                && !operand.is(Ty::TsArrayType)
            {
                return self.raise(
                    node.start as usize,
                    "'readonly' type modifier is only permitted on array and tuple literal types.",
                );
            }
            return Ok(self.finish_node(node, Ty::TsType));
        }
        if self.is_contextual("infer") {
            let node = self.start_node();
            self.expect_contextual("infer")?;
            self.parse_ident(false)?;
            self.ts_try_parse(Self::ts_parse_constraint_for_infer_type)?;
            return Ok(self.finish_node(node, Ty::TsType));
        }
        self.ts_in_disallow_conditional_types_context(false, Self::ts_parse_array_type_or_higher)
    }

    fn ts_parse_union_or_intersection_type(
        &mut self,
        operator: Tok,
        parse_constituent: fn(&mut Self) -> PResult<Node>,
    ) -> PResult<Node> {
        let node = self.start_node();
        let has_leading_operator = self.eat(operator)?;
        let mut types = Vec::new();
        loop {
            types.push(parse_constituent(self)?);
            if !self.eat(operator)? {
                break;
            }
        }
        if types.len() == 1 && !has_leading_operator {
            return Ok(types.remove(0));
        }
        Ok(self.finish_node(node, Ty::TsType))
    }

    fn ts_parse_intersection_type_or_higher(&mut self) -> PResult<Node> {
        self.ts_parse_union_or_intersection_type(
            Tok::BitwiseAnd,
            Self::ts_parse_type_operator_or_higher,
        )
    }

    fn ts_parse_union_type_or_higher(&mut self) -> PResult<Node> {
        self.ts_parse_union_or_intersection_type(
            Tok::BitwiseOr,
            Self::ts_parse_intersection_type_or_higher,
        )
    }

    fn ts_parse_non_conditional_type(&mut self) -> PResult<Node> {
        if self.ts_is_start_of_function_type()? {
            return self.ts_parse_function_or_constructor_type(false, false);
        }
        if self.ty == Tok::KwNew {
            return self.ts_parse_function_or_constructor_type(true, false);
        }
        if self.ts_is_contextual(Tok::TsAbstract) && self.lookahead(1)?.ty == Tok::KwNew {
            return self.ts_parse_function_or_constructor_type(true, true);
        }
        self.ts_parse_union_type_or_higher()
    }

    /// A type.
    pub(crate) fn ts_parse_type(&mut self) -> PResult<Node> {
        let checked = self.ts_parse_non_conditional_type()?;
        if self.in_disallow_conditional_types_context
            || self.has_preceding_line_break()
            || !self.eat(Tok::KwExtends)?
        {
            return Ok(checked);
        }
        let node = self.start_node_at(checked.start as usize);
        self.ts_in_disallow_conditional_types_context(true, Self::ts_parse_non_conditional_type)?;
        self.expect(Tok::Question)?;
        self.ts_in_disallow_conditional_types_context(false, Self::ts_parse_type)?;
        self.expect(Tok::Colon)?;
        self.ts_in_disallow_conditional_types_context(false, Self::ts_parse_type)?;
        Ok(self.finish_node(node, Ty::TsType))
    }
}
