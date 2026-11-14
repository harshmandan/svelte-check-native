//! acorn-typescript's versions of acorn's statement methods, and the
//! declarations only TypeScript has.

use crate::state::{
    Bind, ForInit, Lit, Node, OuterKind, PResult, Parser, PrivateNames, SCOPE_TOP, TS_SCOPE_OTHER,
    TS_SCOPE_TS_MODULE, Ty, function_flags,
};
use crate::statement::{
    ClassPosition, FUNC_HANGING_STATEMENT, FUNC_NULLABLE_ID, FUNC_STATEMENT, check_key_name,
    is_private_name_conflicted,
};
use crate::token::Tok;
use crate::ts::{ListKind, ModifierRules, invalid_modifier_on_type_parameter_positions};
use crate::ts_expression::BodyOf;
use crate::ts_types::TypeParameterModifiers;

/// A declaration acorn-typescript read, or the node handed back where
/// what stands there is none.
type Declared = Result<Node, Node>;

const CLASS_MEMBER_MODIFIERS: &[&str] = &[
    "declare",
    "private",
    "public",
    "protected",
    "accessor",
    "override",
    "abstract",
    "readonly",
    "static",
];

fn is_ambient_numeric_unary_expression(expression: &Node) -> bool {
    expression.is(Ty::UnaryExpression)
        && (expression.name == "-" || expression.name == "+")
        && expression.first().is(Ty::Literal)
        && matches!(expression.first().lit, Lit::Number | Lit::BigInt)
}

fn is_uncomputed_member_expression_chain(expression: &Node) -> bool {
    if expression.is(Ty::Identifier) {
        return true;
    }
    expression.is(Ty::MemberExpression)
        && !expression.computed
        && is_uncomputed_member_expression_chain(expression.first())
}

fn is_possibly_literal_enum(expression: &Node) -> bool {
    if !expression.is(Ty::MemberExpression) {
        return false;
    }
    let property = expression.second();
    if expression.computed && (!property.is(Ty::TemplateLiteral) || !property.list.is_empty()) {
        return false;
    }
    is_uncomputed_member_expression_chain(expression.first())
}

impl Parser {
    // --- statements ---

    /// What acorn-typescript's `parseStatement` reads itself: leading
    /// decorators, an enum, an interface.
    pub(crate) fn ts_parse_statement_start(&mut self) -> PResult<Option<Node>> {
        if self.ty == Tok::At {
            self.parse_decorators(true)?;
        }
        if self.ty == Tok::KwConst && self.is_lookahead_contextual("enum") {
            let node = self.start_node();
            self.expect(Tok::KwConst)?;
            return self.ts_parse_enum_declaration(node, true).map(Some);
        }
        if self.ts_is_contextual(Tok::TsEnum) {
            let node = self.start_node();
            return self.ts_parse_enum_declaration(node, false).map(Some);
        }
        if self.ts_is_contextual(Tok::TsInterface) {
            let node = self.start_node();
            if let Ok(declaration) = self.ts_parse_interface_declaration(node)? {
                return Ok(Some(declaration));
            }
        }
        Ok(None)
    }

    /// An expression statement that is one name may be the start of a
    /// declaration: `declare …`, `global { … }`, `abstract class`,
    /// `module`, `namespace`, `type`.
    pub(crate) fn ts_parse_expression_statement(
        &mut self,
        node: Node,
        name: &str,
    ) -> PResult<Declared> {
        match name {
            "declare" => self.ts_try_parse_declare(node),
            "global" => {
                if self.ty == Tok::BraceL {
                    self.enter_scope(TS_SCOPE_TS_MODULE);
                    self.ts_parse_module_block()?;
                    self.exit_scope();
                    return Ok(Ok(self.finish_node(node, Ty::TsModuleDeclaration)));
                }
                Ok(Err(node))
            }
            _ => self.ts_parse_declaration(node, name, false),
        }
    }

    fn ts_try_parse_declare(&mut self, node: Node) -> PResult<Declared> {
        if self.is_line_terminator()? {
            return Ok(Err(node));
        }
        let mut start_type = self.ty;
        let mut kind = None;
        if self.is_contextual("let") {
            start_type = Tok::KwVar;
            kind = Some("let".to_string());
        }
        self.ts_in_ambient_context(|p| {
            let mut node = node;
            match start_type {
                Tok::KwFunction => {
                    p.next()?;
                    return p
                        .parse_function(node, FUNC_STATEMENT, false, false, ForInit::No)
                        .map(Ok);
                }
                Tok::KwClass => return p.parse_class(node, ClassPosition::Statement).map(Ok),
                Tok::TsEnum => return p.ts_parse_enum_declaration(node, false).map(Ok),
                Tok::TsGlobal => {
                    return p.ts_parse_ambient_external_module_declaration(node).map(Ok);
                }
                Tok::KwConst | Tok::KwVar => {
                    if p.ty != Tok::KwConst || !p.is_lookahead_contextual("enum") {
                        let kind = kind.unwrap_or_else(|| p.value.clone());
                        return p.parse_var_statement(node, &kind, true).map(Ok);
                    }
                    p.expect(Tok::KwConst)?;
                    return p.ts_parse_enum_declaration(node, true).map(Ok);
                }
                Tok::TsInterface => match p.ts_parse_interface_declaration(node)? {
                    Ok(declaration) => return Ok(Ok(declaration)),
                    Err(back) => node = back,
                },
                _ => {}
            }
            if start_type.is_identifier() {
                let value = p.value.clone();
                return p.ts_parse_declaration(node, &value, true);
            }
            Ok(Err(node))
        })
    }

    /// Whether the word just met may start its declaration here. With
    /// `next` the parser still stands on the word and steps over it.
    fn ts_check_line_terminator(&mut self, next: bool) -> PResult<bool> {
        if next {
            if self.has_following_line_break() {
                return Ok(false);
            }
            self.next()?;
            return Ok(true);
        }
        Ok(!self.is_line_terminator()?)
    }

    fn ts_parse_declaration(&mut self, node: Node, value: &str, next: bool) -> PResult<Declared> {
        match value {
            "abstract" => {
                if self.ts_check_line_terminator(next)?
                    && (self.ty == Tok::KwClass || self.ty.is_identifier())
                {
                    return self.ts_parse_abstract_declaration(node);
                }
            }
            "module" => {
                if self.ts_check_line_terminator(next)? {
                    if self.ty == Tok::String {
                        return self
                            .ts_parse_ambient_external_module_declaration(node)
                            .map(Ok);
                    } else if self.ty.is_identifier() {
                        return self
                            .ts_parse_module_or_namespace_declaration(node, false)
                            .map(Ok);
                    }
                }
            }
            "namespace" => {
                if self.ts_check_line_terminator(next)? && self.ty.is_identifier() {
                    return self
                        .ts_parse_module_or_namespace_declaration(node, false)
                        .map(Ok);
                }
            }
            "type" => {
                if self.ts_check_line_terminator(next)? && self.ty.is_identifier() {
                    return self.ts_parse_type_alias_declaration(node).map(Ok);
                }
            }
            _ => {}
        }
        Ok(Err(node))
    }

    fn ts_parse_abstract_declaration(&mut self, mut node: Node) -> PResult<Declared> {
        if self.ty == Tok::KwClass {
            node.modifiers.push("abstract".to_string());
            return self.parse_class(node, ClassPosition::Statement).map(Ok);
        }
        if self.ts_is_contextual(Tok::TsInterface) {
            if !self.has_following_line_break() {
                return self.ts_parse_interface_declaration(node);
            }
            return Ok(Err(node));
        }
        self.unexpected_at(node.start as usize)
    }

    fn ts_parse_interface_declaration(&mut self, mut node: Node) -> PResult<Declared> {
        if self.has_following_line_break() {
            return Ok(Err(node));
        }
        self.expect_contextual("interface")?;
        if self.ty.is_identifier() {
            let id = self.parse_ident(false)?;
            self.check_lval_simple(&id, Bind::TsInterface, None)?;
            if self.current_scope().flags & SCOPE_TOP != 0 {
                self.forget_undefined_export(&id.name);
            }
            node.a = Some(Box::new(id));
        } else {
            return self.raise(
                self.start,
                "'interface' declarations must be followed by an identifier.",
            );
        }
        self.ts_try_parse_type_parameters(TypeParameterModifiers::InOut)?;
        if self.eat(Tok::KwExtends)? {
            self.ts_parse_heritage_clause("extends")?;
        }
        self.expect(Tok::BraceL)?;
        let old_in_type = self.in_type;
        self.in_type = true;
        let members = self.ts_parse_list(ListKind::TypeMembers, Self::ts_parse_type_member);
        self.in_type = old_in_type;
        members?;
        self.expect(Tok::BraceR)?;
        Ok(Ok(self.finish_node(node, Ty::TsInterfaceDeclaration)))
    }

    fn ts_parse_type_alias_declaration(&mut self, mut node: Node) -> PResult<Node> {
        let id = self.parse_ident(false)?;
        self.check_lval_simple(&id, Bind::TsType, None)?;
        if self.current_scope().flags & SCOPE_TOP != 0 {
            self.forget_undefined_export(&id.name);
        }
        node.a = Some(Box::new(id));
        self.ts_in_type(|p| {
            p.ts_try_parse_type_parameters(TypeParameterModifiers::InOut)?;
            p.expect(Tok::Eq)?;
            if p.ts_is_contextual(Tok::TsInterface) && p.lookahead(1)?.ty != Tok::Dot {
                // `intrinsic`-style placeholder: one word and no more.
                return p.next();
            }
            p.ts_parse_type().map(|_| ())
        })?;
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::TsTypeAliasDeclaration))
    }

    fn ts_parse_enum_member(&mut self) -> PResult<()> {
        if self.ty == Tok::String {
            self.parse_literal()?;
        } else {
            self.parse_ident(true)?;
        }
        if self.eat(Tok::Eq)? {
            self.parse_maybe_assign(ForInit::No, None)?;
        }
        Ok(())
    }

    fn ts_parse_enum_declaration(&mut self, mut node: Node, is_const: bool) -> PResult<Node> {
        self.expect_contextual("enum")?;
        let id = self.parse_ident(false)?;
        self.check_lval_simple(
            &id,
            if is_const {
                Bind::TsConstEnum
            } else {
                Bind::TsEnum
            },
            None,
        )?;
        node.a = Some(Box::new(id));
        self.expect(Tok::BraceL)?;
        self.ts_parse_delimited_list(ListKind::EnumMembers, Self::ts_parse_enum_member)?;
        self.expect(Tok::BraceR)?;
        Ok(self.finish_node(node, Ty::TsEnumDeclaration))
    }

    fn ts_parse_module_block(&mut self) -> PResult<()> {
        self.enter_scope(TS_SCOPE_OTHER);
        self.expect(Tok::BraceL)?;
        while self.ty != Tok::BraceR {
            self.parse_statement(None, true)?;
        }
        self.next()?;
        self.exit_scope();
        Ok(())
    }

    fn ts_parse_module_or_namespace_declaration(
        &mut self,
        mut node: Node,
        nested: bool,
    ) -> PResult<Node> {
        let id = self.parse_ident(false)?;
        if !nested {
            self.check_lval_simple(&id, Bind::TsExportOnly, None)?;
        }
        node.a = Some(Box::new(id));
        if self.eat(Tok::Dot)? {
            let inner = self.start_node();
            self.ts_parse_module_or_namespace_declaration(inner, true)?;
        } else {
            self.enter_scope(TS_SCOPE_TS_MODULE);
            self.ts_parse_module_block()?;
            self.exit_scope();
        }
        Ok(self.finish_node(node, Ty::TsModuleDeclaration))
    }

    fn ts_parse_ambient_external_module_declaration(&mut self, mut node: Node) -> PResult<Node> {
        if self.ts_is_contextual(Tok::TsGlobal) {
            node.a = Some(Box::new(self.parse_ident(false)?));
        } else if self.ty == Tok::String {
            node.a = Some(Box::new(self.parse_literal()?));
        } else {
            return self.unexpected();
        }
        if self.ty == Tok::BraceL {
            self.enter_scope(TS_SCOPE_TS_MODULE);
            self.ts_parse_module_block()?;
            self.exit_scope();
        } else {
            self.semicolon()?;
        }
        Ok(self.finish_node(node, Ty::TsModuleDeclaration))
    }

    /// acorn-typescript's `parseVarStatement`: an ambient declaration
    /// needs no initializer and takes only a literal one.
    pub(crate) fn ts_parse_var_statement(
        &mut self,
        mut node: Node,
        kind: &str,
        allow_missing_initializer: bool,
    ) -> PResult<Node> {
        let is_ambient = self.is_ambient_context;
        self.next()?;
        self.parse_var(
            &mut node,
            false,
            kind,
            allow_missing_initializer || is_ambient,
        )?;
        self.semicolon()?;
        let declaration = self.finish_node(node, Ty::VariableDeclaration);
        if !is_ambient {
            return Ok(declaration);
        }
        for declarator in declaration.list.iter().flatten() {
            let Some(init) = declarator.b.as_deref() else {
                continue;
            };
            if kind != "const" || declarator.first().annotation.is_some() {
                return self.raise(
                    init.start as usize,
                    "Initializers are not allowed in ambient contexts.",
                );
            }
            let literal = init.is(Ty::Literal)
                || (init.is(Ty::TemplateLiteral) && init.list.is_empty())
                || is_ambient_numeric_unary_expression(init)
                || is_possibly_literal_enum(init);
            if !literal {
                return self.raise(
                    init.start as usize,
                    "A 'const' initializer in an ambient context must be a string or numeric literal or literal enum reference.",
                );
            }
        }
        Ok(declaration)
    }

    /// What may follow a declared name: `!`, and a type annotation.
    pub(crate) fn ts_after_var_id(&mut self, id: &mut Node) -> PResult<()> {
        if id.is(Ty::Identifier)
            && !self.has_preceding_line_break()
            && self.value == "!"
            && self.ty == Tok::Prefix
        {
            self.next()?;
        }
        if let Some(annotation) = self.ts_try_parse_type_annotation()? {
            id.annotation = Some(annotation.start);
            id.end = self.last_tok_end as u32;
        }
        Ok(())
    }

    // --- functions ---

    /// acorn-typescript's `parseFunction`. It checks the function's
    /// name after its body, because a declaration without a body binds
    /// nothing.
    pub(crate) fn ts_parse_function(
        &mut self,
        mut node: Node,
        statement: u8,
        allow_expression_body: bool,
        is_async: bool,
        for_init: ForInit,
    ) -> PResult<Node> {
        if self.ty == Tok::Star && statement & FUNC_HANGING_STATEMENT != 0 {
            return self.unexpected();
        }
        let generator = self.eat(Tok::Star)?;
        let is_declaration = statement & FUNC_STATEMENT != 0;
        if is_declaration {
            // Only a plain name: one of TypeScript's own words is none.
            node.a = if statement & FUNC_NULLABLE_ID != 0 && self.ty != Tok::Name {
                None
            } else {
                Some(Box::new(self.parse_ident(false)?))
            };
        }

        let old_yield_pos = self.yield_pos;
        let old_await_pos = self.await_pos;
        let old_await_ident_pos = self.await_ident_pos;
        let old_maybe_in_arrow_parameters = self.maybe_in_arrow_parameters;
        self.maybe_in_arrow_parameters = false;
        self.yield_pos = 0;
        self.await_pos = 0;
        self.await_ident_pos = 0;
        self.enter_scope(function_flags(is_async, generator));

        if !is_declaration && self.ty == Tok::Name {
            node.a = Some(Box::new(self.parse_ident(false)?));
        }

        self.ts_try_parse_type_parameters(TypeParameterModifiers::Const)?;
        self.expect(Tok::ParenL)?;
        node.list = self.parse_binding_list(Tok::ParenR, false, true, None)?;
        self.check_yield_await_in_default_params()?;
        self.parse_function_body(
            &mut node,
            allow_expression_body,
            false,
            for_init,
            if is_declaration {
                BodyOf::FunctionDeclaration
            } else {
                BodyOf::Other
            },
        )?;

        let id = node.a.as_deref().cloned();
        if is_declaration
            && self.is_ambient_context
            && let Some(id) = &id
            && self.current_scope().flags & SCOPE_TOP != 0
        {
            self.declare_name(&id.name, Bind::TsExportOnly, id.start)?;
        }

        self.yield_pos = old_yield_pos;
        self.await_pos = old_await_pos;
        self.await_ident_pos = old_await_ident_pos;

        if is_declaration
            && statement & FUNC_HANGING_STATEMENT == 0
            && let Some(id) = &id
        {
            let binding = if !node.has_body {
                Bind::None
            } else if self.strict || generator || is_async {
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

        self.maybe_in_arrow_parameters = old_maybe_in_arrow_parameters;
        Ok(self.finish_node(
            node,
            if is_declaration {
                Ty::FunctionDeclaration
            } else {
                Ty::FunctionExpression
            },
        ))
    }

    // --- classes ---

    /// acorn-typescript's `parseClass`.
    pub(crate) fn ts_parse_class(&mut self, node: Node, position: ClassPosition) -> PResult<Node> {
        let old_in_abstract_class = self.in_abstract_class;
        self.in_abstract_class = node.has_modifier("abstract");
        let result = self.ts_parse_class_inner(node, position);
        self.in_abstract_class = old_in_abstract_class;
        result
    }

    fn ts_parse_class_inner(&mut self, mut node: Node, position: ClassPosition) -> PResult<Node> {
        self.next()?;
        self.take_decorators();
        let old_strict = self.strict;
        self.strict = true;

        // An unnamed class expression that goes straight to
        // `implements` has no name to read.
        if !(position == ClassPosition::Expression && self.is_contextual("implements")) {
            if self.ty == Tok::Name {
                let id = self.parse_ident(false)?;
                if position != ClassPosition::Expression {
                    self.check_lval_simple(&id, Bind::Lexical, None)?;
                }
                node.a = Some(Box::new(id));
            } else if position == ClassPosition::Statement {
                return self.unexpected();
            }
            self.ts_try_parse_type_parameters(TypeParameterModifiers::Class)?;
        }

        let has_super_class = if self.eat(Tok::KwExtends)? {
            self.parse_expr_subscripts(None, ForInit::No)?;
            true
        } else {
            false
        };
        if has_super_class && (self.ts_match_left_relational() || self.ty == Tok::BitShift) {
            self.ts_parse_type_arguments_in_expression()?;
        }
        if self.eat_contextual("implements")? {
            self.ts_parse_heritage_clause("implements")?;
        }

        self.private_name_stack.push(PrivateNames {
            declared: Vec::new(),
            used: Vec::new(),
        });
        let mut had_constructor = false;
        // The decorators before the next member, and where the first
        // starts: a decorated member starts there.
        let mut decorators = 0;
        let mut first_decorator = 0;
        self.expect(Tok::BraceL)?;
        while self.ty != Tok::BraceR {
            if self.ty == Tok::At {
                if decorators == 0 {
                    first_decorator = self.start as u32;
                }
                self.parse_decorator()?;
                decorators += 1;
                continue;
            }
            let Some(mut element) = self.ts_parse_class_element(has_super_class)? else {
                continue;
            };
            let decorated = decorators > 0;
            if decorated {
                element.start = first_decorator;
                decorators = 0;
            }
            if element.is(Ty::MethodDefinition)
                && element.name == "constructor"
                && element.second().is(Ty::FunctionExpression)
            {
                if had_constructor {
                    return self.raise(
                        element.start as usize,
                        "Duplicate constructor in the same class",
                    );
                }
                had_constructor = true;
                if decorated {
                    return self.raise(
                        element.start as usize,
                        "Decorators can't be used with a constructor. Did you mean '@dec class { ... }'?",
                    );
                }
            } else if element.a.is_some()
                && element.first().is(Ty::PrivateIdentifier)
                && !element
                    .b
                    .as_deref()
                    .is_some_and(|value| value.is(Ty::TsDeclareMethod))
            {
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
        if decorators > 0 {
            return self.raise(
                self.start,
                "Decorators must be attached to a class element.",
            );
        }
        self.exit_class_body()?;
        Ok(self.finish_node(
            node,
            if position == ClassPosition::Expression {
                Ty::ClassExpression
            } else {
                Ty::ClassDeclaration
            },
        ))
    }

    fn ts_parse_class_element(&mut self, constructor_allows_super: bool) -> PResult<Option<Node>> {
        if self.eat(Tok::Semi)? {
            return Ok(None);
        }
        let mut node = self.start_node();
        let read = self.ts_parse_modifiers(
            &mut node,
            &ModifierRules {
                allowed: CLASS_MEMBER_MODIFIERS,
                disallowed: &["in", "out"],
                stop_on_start_of_class_static_block: true,
                error: invalid_modifier_on_type_parameter_positions,
            },
        )?;
        let is_static = read.iter().any(|m| m == "static");
        if node.has_modifier("declare") {
            self.ts_in_ambient_context(|p| {
                p.ts_parse_class_member(node, is_static, constructor_allows_super)
            })
            .map(Some)
        } else {
            self.ts_parse_class_member(node, is_static, constructor_allows_super)
                .map(Some)
        }
    }

    fn ts_parse_class_member(
        &mut self,
        mut node: Node,
        is_static: bool,
        constructor_allows_super: bool,
    ) -> PResult<Node> {
        if self.ts_is_start_of_static_blocks() {
            self.next()?;
            self.next()?;
            if node.accessibility.is_some() || !node.modifiers.is_empty() {
                return self.raise(self.start, "Static class blocks cannot have any modifier.");
            }
            self.parse_class_static_block()?;
            return Ok(self.finish_node(node, Ty::StaticBlock));
        }

        if self.ts_try_parse_index_signature(&mut node)? {
            if node.has_modifier("abstract") {
                return self.raise(
                    node.start as usize,
                    "Index signatures cannot have the 'abstract' modifier.",
                );
            }
            if let Some(modifier) = &node.accessibility {
                return self.raise(
                    node.start as usize,
                    format!(
                        "Index signatures cannot have an accessibility modifier ('{modifier}')."
                    ),
                );
            }
            if node.has_modifier("declare") {
                return self.raise(
                    node.start as usize,
                    "Index signatures cannot have the 'declare' modifier.",
                );
            }
            if node.has_modifier("override") {
                return self.raise(
                    node.start as usize,
                    "'override' modifier cannot appear on an index signature.",
                );
            }
            return Ok(node);
        }

        if !self.in_abstract_class && node.has_modifier("abstract") {
            return self.raise(
                node.start as usize,
                "Abstract methods can only appear within an abstract class.",
            );
        }
        if node.has_modifier("override") && !constructor_allows_super {
            return self.raise(
                node.start as usize,
                "This member cannot have an 'override' modifier because its containing class does not extend another class.",
            );
        }

        let mut key_name = String::new();
        let mut is_generator = false;
        let mut is_async = false;
        let mut kind = "method".to_string();
        node.is_static = is_static;
        if is_static && !(self.is_class_element_name_start() || self.ty == Tok::Star) {
            key_name = "static".to_string();
        }
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
            node.computed = false;
            let mut key = self.start_node_at(self.last_tok_start);
            key.name = key_name;
            node.a = Some(Box::new(self.finish_node(key, Ty::Identifier)));
        }

        // parsePostMemberNameModifiers
        if self.eat(Tok::Question)? {
            node.optional = true;
        }
        if node.has_modifier("readonly") && self.ty == Tok::ParenL {
            return self.raise(
                node.start as usize,
                "Class methods cannot have the 'readonly' modifier.",
            );
        }
        if node.has_modifier("declare") && self.ty == Tok::ParenL {
            return self.raise(
                node.start as usize,
                "Class methods cannot have the 'declare' modifier.",
            );
        }

        // A `<` or `>` after the name starts a method's type parameters.
        if self.ty == Tok::Relational
            || self.ty == Tok::ParenL
            || kind != "method"
            || is_generator
            || is_async
        {
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
            self.ts_parse_class_method(node, is_generator, is_async, allows_direct_super)
        } else {
            self.ts_parse_class_field(node)
        }
    }

    fn ts_parse_class_method(
        &mut self,
        mut method: Node,
        is_generator: bool,
        is_async: bool,
        allows_direct_super: bool,
    ) -> PResult<Node> {
        let is_constructor = method.name == "constructor";
        let is_private = method.first().is(Ty::PrivateIdentifier);
        let type_parameters = self.ts_try_parse_type_parameters(TypeParameterModifiers::Const)?;
        if is_private {
            if let Some(modifier) = &method.accessibility {
                return self.raise(
                    method.start as usize,
                    format!(
                        "Private methods cannot have an accessibility modifier ('{modifier}')."
                    ),
                );
            }
        } else if let Some(type_parameters) = &type_parameters
            && is_constructor
        {
            return self.raise(
                type_parameters.start as usize,
                "Type parameters cannot appear on a constructor declaration.",
            );
        }
        if method.has_modifier("declare") && (method.name == "get" || method.name == "set") {
            return self.raise(
                method.start as usize,
                format!("'declare' is not allowed in {}ters.", method.name),
            );
        }

        let key_start = method.first().start as usize;
        if is_constructor {
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

        let value = self.ts_parse_method(
            is_generator,
            is_async,
            allows_direct_super,
            true,
            Some(&method),
        )?;
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

    fn ts_parse_class_property_annotation(&mut self, node: &mut Node) -> PResult<()> {
        if !node.optional {
            if self.value == "!" && self.ty == Tok::Prefix {
                self.next()?;
            } else if self.eat(Tok::Question)? {
                node.optional = true;
            }
        }
        if let Some(annotation) = self.ts_try_parse_type_annotation()? {
            node.annotation = Some(annotation.start);
        }
        Ok(())
    }

    fn ts_parse_class_field(&mut self, mut field: Node) -> PResult<Node> {
        if field.first().is(Ty::PrivateIdentifier) {
            if field.has_modifier("abstract") {
                return self.raise(
                    field.start as usize,
                    "Private elements cannot have the 'abstract' modifier.",
                );
            }
            if let Some(modifier) = &field.accessibility {
                return self.raise(
                    field.start as usize,
                    format!(
                        "Private elements cannot have an accessibility modifier ('{modifier}')."
                    ),
                );
            }
            self.ts_parse_class_property_annotation(&mut field)?;
        } else {
            self.ts_parse_class_property_annotation(&mut field)?;
            if self.is_ambient_context
                && !(field.has_modifier("readonly") && field.annotation.is_none())
                && self.ty == Tok::Eq
            {
                return self.raise(
                    self.start,
                    "Initializers are not allowed in ambient contexts.",
                );
            }
            if field.has_modifier("abstract") && self.ty == Tok::Eq {
                let key = field.first();
                let name = if key.is(Ty::Identifier) && !field.computed {
                    key.name.clone()
                } else {
                    format!("[{}]", self.slice(key.start as usize, key.end as usize))
                };
                return self.raise(
                    self.start,
                    format!(
                        "Property '{name}' cannot have an initializer because it is marked abstract."
                    ),
                );
            }
        }
        self.parse_class_field(field)
    }

    // --- imports and exports ---

    fn ts_parse_import_equals_declaration(&mut self, node: Node, type_only: bool) -> PResult<Node> {
        let id = self.parse_ident(false)?;
        self.check_lval_simple(&id, Bind::Lexical, None)?;
        self.expect(Tok::Eq)?;
        // `require("…")`, or a name.
        if self.is_contextual("require") && self.lookahead_char_code() == 40 {
            self.expect_contextual("require")?;
            self.expect(Tok::ParenL)?;
            if self.ty != Tok::String {
                return self.unexpected();
            }
            self.parse_expr_atom(None, ForInit::No, false)?;
            self.expect(Tok::ParenR)?;
        } else {
            let reference = self.ts_parse_entity_name(false)?;
            if type_only {
                return self.raise(
                    reference.start as usize,
                    "An import alias can not use 'import type'.",
                );
            }
        }
        self.semicolon()?;
        Ok(self.finish_node(node, Ty::Other))
    }

    /// `type` before or inside an import or export specifier: which of
    /// the words around `as` is the name, and which the modifier.
    fn ts_parse_type_only_import_export_specifier(
        &mut self,
        mut left: Node,
        is_import: bool,
        in_type_only: bool,
    ) -> PResult<(Node, Node)> {
        let loc = left.start as usize;
        let mut right = None;
        let mut has_type_specifier = false;
        let mut can_parse_as_keyword = true;

        if self.is_contextual("as") {
            let first_as = self.parse_ident(false)?;
            if self.is_contextual("as") {
                let second_as = self.parse_ident(false)?;
                if self.ty.is_keyword_or_identifier() {
                    has_type_specifier = true;
                    left = first_as;
                    right = Some(self.ts_parse_specifier_name(is_import)?);
                } else {
                    right = Some(second_as);
                }
                can_parse_as_keyword = false;
            } else if self.ty.is_keyword_or_identifier() {
                can_parse_as_keyword = false;
                right = Some(self.ts_parse_specifier_name(is_import)?);
            } else {
                has_type_specifier = true;
                left = first_as;
            }
        } else if self.ty.is_keyword_or_identifier() {
            has_type_specifier = true;
            if is_import {
                left = self.parse_ident(true)?;
                if !self.is_contextual("as") {
                    self.check_unreserved(left.start as usize, &left.name)?;
                }
            } else {
                left = self.parse_module_export_name()?;
            }
        }
        if has_type_specifier && in_type_only {
            return self.raise(
                loc,
                if is_import {
                    "The 'type' modifier cannot be used on a named import when 'import type' is used on its import statement."
                } else {
                    "The 'type' modifier cannot be used on a named export when 'export type' is used on its export statement."
                },
            );
        }
        if can_parse_as_keyword && self.eat_contextual("as")? {
            right = Some(self.ts_parse_specifier_name(is_import)?);
        }
        let right = right.unwrap_or_else(|| left.clone());
        if is_import {
            self.check_lval_simple(&right, Bind::Lexical, None)?;
        }
        Ok((left, right))
    }

    fn ts_parse_specifier_name(&mut self, is_import: bool) -> PResult<Node> {
        if is_import {
            self.parse_ident(false)
        } else {
            self.parse_module_export_name()
        }
    }

    fn ts_parse_import_specifier(&mut self) -> PResult<()> {
        if self.ts_is_contextual(Tok::TsType) {
            let imported = self.parse_module_export_name()?;
            let in_type_only = self.import_or_export_outer_kind == OuterKind::Type;
            self.ts_parse_type_only_import_export_specifier(imported, true, in_type_only)?;
            return Ok(());
        }
        let imported = self.parse_module_export_name()?;
        let local = if self.eat_contextual("as")? {
            self.parse_ident(false)?
        } else {
            self.check_unreserved(imported.start as usize, &imported.name)?;
            imported
        };
        self.check_lval_simple(&local, Bind::Lexical, None)
    }

    /// The specifiers of an import: how many, and whether the first is
    /// the default import.
    fn ts_parse_import_specifiers(&mut self) -> PResult<(usize, bool)> {
        let mut count = 0;
        let mut starts_with_default = false;
        if self.ty.is_identifier() {
            let local = self.parse_ident(false)?;
            self.check_lval_simple(&local, Bind::Lexical, None)?;
            count += 1;
            starts_with_default = true;
            if !self.eat(Tok::Comma)? {
                return Ok((count, starts_with_default));
            }
        }
        if self.ty == Tok::Star {
            self.next()?;
            self.expect_contextual("as")?;
            let local = self.parse_ident(false)?;
            self.check_lval_simple(&local, Bind::Lexical, None)?;
            return Ok((count + 1, starts_with_default));
        }
        self.expect(Tok::BraceL)?;
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
            self.ts_parse_import_specifier()?;
            count += 1;
        }
        Ok((count, starts_with_default))
    }

    /// acorn-typescript's `parseImport`.
    pub(crate) fn ts_parse_import(&mut self, node: Node) -> PResult<Node> {
        let mut enter_head = self.lookahead(1)?;
        let mut type_only = false;
        self.import_or_export_outer_kind = OuterKind::Value;
        if enter_head.ty.is_identifier() || self.ty == Tok::Star || self.ty == Tok::BraceL {
            let mut ahead = self.lookahead(2)?;
            let ahead_is_from =
                ahead.ty == Tok::Name && ahead.value == "from" && !ahead.contains_esc;
            if ahead.ty != Tok::Comma
                && !ahead_is_from
                && ahead.ty != Tok::Eq
                && enter_head.ty == Tok::TsType
                && !enter_head.contains_esc
            {
                self.next()?;
                self.import_or_export_outer_kind = OuterKind::Type;
                type_only = true;
                enter_head = self.lookahead(1)?;
                ahead = self.lookahead(2)?;
            }
            if enter_head.ty.is_identifier() && ahead.ty == Tok::Eq {
                self.next()?;
                let declaration = self.ts_parse_import_equals_declaration(node, type_only)?;
                self.import_or_export_outer_kind = OuterKind::Value;
                return Ok(declaration);
            }
        }
        self.next()?;
        let mut specifiers = (0, false);
        if self.ty == Tok::String {
            self.parse_expr_atom(None, ForInit::No, false)?;
        } else {
            specifiers = self.ts_parse_import_specifiers()?;
            self.expect_contextual("from")?;
            if self.ty != Tok::String {
                return self.unexpected();
            }
            self.parse_expr_atom(None, ForInit::No, false)?;
        }
        self.parse_maybe_import_attributes()?;
        self.semicolon()?;
        let node = self.finish_node(node, Ty::Other);
        self.import_or_export_outer_kind = OuterKind::Value;
        if type_only && specifiers.0 > 1 && specifiers.1 {
            return self.raise(
                node.start as usize,
                "A type-only import can specify a default import or named bindings, but not both.",
            );
        }
        Ok(node)
    }

    fn ts_parse_export_specifiers(&mut self) -> PResult<Vec<Node>> {
        let mut locals = Vec::new();
        let mut first = true;
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
            if self.ty != Tok::String && self.ts_is_contextual(Tok::TsType) {
                let local = self.parse_module_export_name()?;
                let in_type_only = self.import_or_export_outer_kind == OuterKind::Type;
                let (local, _) =
                    self.ts_parse_type_only_import_export_specifier(local, false, in_type_only)?;
                locals.push(local);
            } else {
                let local = self.parse_module_export_name()?;
                if self.eat_contextual("as")? {
                    self.parse_module_export_name()?;
                }
                locals.push(local);
            }
        }
        Ok(locals)
    }

    fn ts_parse_export_declaration(&mut self) -> PResult<Node> {
        if !self.is_ambient_context && self.ts_is_contextual(Tok::TsDeclare) {
            return self.ts_in_ambient_context(Self::ts_parse_export_declaration);
        }
        let is_declare = self.eat_contextual("declare")?;
        if is_declare
            && (self.ts_is_contextual(Tok::TsDeclare) || !self.should_parse_export_statement())
        {
            return self.raise(
                self.start,
                "'export declare' must be followed by an ambient declaration.",
            );
        }
        if self.ty.is_identifier() {
            let node = self.start_node();
            let value = self.value.clone();
            if let Ok(declaration) = self.ts_parse_declaration(node, &value, true)? {
                return Ok(declaration);
            }
        }
        self.parse_statement(None, false)
    }

    /// acorn-typescript's `parseExport`.
    pub(crate) fn ts_parse_export(&mut self, node: Node) -> PResult<Node> {
        let enter_head = self.lookahead(1)?;
        if enter_head.ty == Tok::KwImport {
            // export import a = b
            self.next()?;
            self.next()?;
            let type_only = self.ts_is_contextual(Tok::TsType) && self.lookahead_char_code() != 61;
            if type_only {
                self.import_or_export_outer_kind = OuterKind::Type;
                self.next()?;
            } else {
                self.import_or_export_outer_kind = OuterKind::Value;
            }
            let declaration = self.ts_parse_import_equals_declaration(node, type_only)?;
            self.import_or_export_outer_kind = OuterKind::Unset;
            return Ok(declaration);
        }
        if enter_head.ty == Tok::Eq {
            // export = value
            self.next()?;
            self.next()?;
            self.parse_expression(ForInit::No, None)?;
            self.semicolon()?;
            self.import_or_export_outer_kind = OuterKind::Unset;
            return Ok(self.finish_node(node, Ty::Other));
        }
        if enter_head.ty == Tok::Name && enter_head.value == "as" && !enter_head.contains_esc {
            // export as namespace A
            self.next()?;
            self.next()?;
            self.expect_contextual("namespace")?;
            self.parse_ident(false)?;
            self.semicolon()?;
            self.import_or_export_outer_kind = OuterKind::Unset;
            return Ok(self.finish_node(node, Ty::Other));
        }

        let after_head = self.lookahead(2)?.ty;
        if enter_head.ty == Tok::TsType
            && !enter_head.contains_esc
            && (after_head == Tok::BraceL || after_head == Tok::Star)
        {
            self.next()?;
            self.import_or_export_outer_kind = OuterKind::Type;
        } else {
            self.import_or_export_outer_kind = OuterKind::Value;
        }
        self.next()?;
        if self.eat(Tok::Star)? {
            if self.eat_contextual("as")? {
                self.parse_module_export_name()?;
            }
            self.expect_contextual("from")?;
            if self.ty != Tok::String {
                return self.unexpected();
            }
            self.parse_expr_atom(None, ForInit::No, false)?;
            self.parse_maybe_import_attributes()?;
            self.semicolon()?;
            return Ok(self.finish_node(node, Ty::Other));
        }
        if self.eat(Tok::KwDefault)? {
            self.ts_parse_export_default_declaration()?;
            return Ok(self.finish_node(node, Ty::Other));
        }
        if self.should_parse_export_statement() {
            self.ts_parse_export_declaration()?;
        } else {
            let locals = self.ts_parse_export_specifiers()?;
            if self.eat_contextual("from")? {
                if self.ty != Tok::String {
                    return self.unexpected();
                }
                self.parse_expr_atom(None, ForInit::No, false)?;
                self.parse_maybe_import_attributes()?;
            } else {
                for local in &locals {
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

    fn ts_parse_export_default_declaration(&mut self) -> PResult<()> {
        if self.ts_is_abstract_class()? {
            let mut class = self.start_node();
            self.next()?;
            class.modifiers.push("abstract".to_string());
            self.parse_class(class, ClassPosition::Statement)?;
            return Ok(());
        }
        if self.ty == Tok::TsInterface {
            let node = self.start_node();
            if self.ts_parse_interface_declaration(node)?.is_ok() {
                return Ok(());
            }
        }
        self.parse_export_default_declaration()
    }
}
