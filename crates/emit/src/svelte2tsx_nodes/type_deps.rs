//! AST-based type-dependency collection for hoistability decisions.
//!
//! Replaces the two hand-rolled `&[u8]` scanners this module used to
//! host (`type_refs.rs`'s `typeof_targets` / `keyof_typeof_targets`
//! and `ident_refs.rs`'s `collect_ident_refs`). Per architecture rule
//! #1, embedded TS type-source is walked at the AST level instead of
//! scanned character-by-character: the caller already has the parsed
//! declaration in hand, so we walk `oxc`'s TS-type AST rather than
//! re-scanning the byte slice.
//!
//! Mirrors the relevant half of upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/HoistableInterfaces.ts`:
//! `collectTypeDependencies` (walks with `ts.forEachChild`, matching
//! `ts.isTypeReferenceNode` → type deps and `ts.isTypeQueryNode` →
//! value/`typeof` deps) rooted via `getEntityNameRoot` (the left-most
//! identifier of a qualified name).
//!
//! Three sets are produced per declaration:
//!
//! - [`TypeDeps::idents`] — every identifier the old `collect_ident_refs`
//!   byte scanner would have surfaced from the declaration text:
//!   type-reference roots, `typeof` targets, property / method / index
//!   keys, parameter names, generic-parameter names, heritage roots,
//!   and the declaration's own name. Used wherever the caller
//!   intersects against a known name set (body-local value names or
//!   declared type names), so over-collection (e.g. a property key) is
//!   harmless — it just won't be in the intersection set. Reproducing
//!   the scanner's breadth keeps the hoist decision byte-identical
//!   (a hoisted `interface Props { foo: T }` whose key `foo` coincides
//!   with the destructured prop local `foo` still emits its
//!   `declare const foo` stub).
//! - [`TypeDeps::typeof_refs`] — roots of every `typeof X` (bare or
//!   under `keyof`). Reproduces `typeof_targets`.
//! - [`TypeDeps::keyof_typeof_refs`] — roots of `typeof X` that sit
//!   directly under a `keyof` operator (parentheses unwrapped).
//!   Reproduces `keyof_typeof_targets`.

use std::collections::HashSet;

use oxc_ast::ast::{
    BindingPattern, Expression, FormalParameters, PropertyKey, TSInterfaceDeclaration,
    TSInterfaceHeritage, TSSignature, TSTupleElement, TSType, TSTypeAliasDeclaration,
    TSTypeAnnotation, TSTypeName, TSTypeOperatorOperator, TSTypeParameterDeclaration,
    TSTypePredicateName, TSTypeQueryExprName,
};
use smol_str::SmolStr;

/// Per-declaration dependency sets collected from the TS-type AST.
#[derive(Debug, Default, Clone)]
pub(crate) struct TypeDeps {
    /// Broad identifier set — reproduces the old `collect_ident_refs`
    /// byte scanner (type-reference roots, `typeof` targets, property /
    /// method / index keys, parameter names, generic names, heritage
    /// roots, and the declaration's own name).
    pub idents: HashSet<SmolStr>,
    /// Roots of every `typeof X` query (bare or under `keyof`).
    pub typeof_refs: HashSet<SmolStr>,
    /// Roots of `typeof X` queries directly under a `keyof` operator.
    pub keyof_typeof_refs: HashSet<SmolStr>,
}

/// Collect deps from a `type Foo<...> = ...` alias declaration.
pub(crate) fn collect_alias_deps(decl: &TSTypeAliasDeclaration<'_>) -> TypeDeps {
    let mut out = TypeDeps::default();
    out.idents.insert(SmolStr::from(decl.id.name.as_str()));
    walk_type_params(decl.type_parameters.as_deref(), &mut out);
    walk(&decl.type_annotation, &mut out);
    out
}

/// Collect deps from an `interface Foo<...> extends ... { ... }`
/// declaration: generic parameters, each `extends` heritage clause
/// (its root identifier plus type arguments) and every member
/// signature.
pub(crate) fn collect_interface_deps(decl: &TSInterfaceDeclaration<'_>) -> TypeDeps {
    let mut out = TypeDeps::default();
    out.idents.insert(SmolStr::from(decl.id.name.as_str()));
    walk_type_params(decl.type_parameters.as_deref(), &mut out);
    for heritage in &decl.extends {
        walk_heritage(heritage, &mut out);
    }
    for sig in &decl.body.body {
        walk_signature(sig, &mut out);
    }
    out
}

/// Collect deps from a bare type node — used for an exported
/// declaration's annotation (e.g. `export let state: Foo | undefined`).
pub(crate) fn collect_type_node_deps(ty: &TSType<'_>) -> TypeDeps {
    let mut out = TypeDeps::default();
    walk(ty, &mut out);
    out
}

/// Exhaustive recursive walk over a TS type node. Every `TSType`
/// variant is matched explicitly (no `_ =>` fallback) so the compiler
/// fails when oxc adds a variant we haven't decided on — the same
/// discipline as `collect_function_body_stmts`'s expression match.
fn walk(ty: &TSType<'_>, out: &mut TypeDeps) {
    match ty {
        // Keywords / literals / `this` — no identifiers to collect.
        TSType::TSAnyKeyword(_)
        | TSType::TSBigIntKeyword(_)
        | TSType::TSBooleanKeyword(_)
        | TSType::TSIntrinsicKeyword(_)
        | TSType::TSNeverKeyword(_)
        | TSType::TSNullKeyword(_)
        | TSType::TSNumberKeyword(_)
        | TSType::TSObjectKeyword(_)
        | TSType::TSStringKeyword(_)
        | TSType::TSSymbolKeyword(_)
        | TSType::TSUndefinedKeyword(_)
        | TSType::TSUnknownKeyword(_)
        | TSType::TSVoidKeyword(_)
        | TSType::TSThisType(_)
        | TSType::TSLiteralType(_)
        | TSType::JSDocUnknownType(_) => {}

        TSType::TSArrayType(t) => walk(&t.element_type, out),
        TSType::TSConditionalType(t) => {
            walk(&t.check_type, out);
            walk(&t.extends_type, out);
            walk(&t.true_type, out);
            walk(&t.false_type, out);
        }
        TSType::TSConstructorType(t) => {
            walk_type_params(t.type_parameters.as_deref(), out);
            walk_params(&t.params, out);
            walk_type_annotation(&t.return_type, out);
        }
        TSType::TSFunctionType(t) => {
            walk_type_params(t.type_parameters.as_deref(), out);
            if let Some(this_param) = &t.this_param
                && let Some(ta) = &this_param.type_annotation
            {
                walk_type_annotation(ta, out);
            }
            walk_params(&t.params, out);
            walk_type_annotation(&t.return_type, out);
        }
        TSType::TSImportType(t) => {
            // `import('mod')` / `import('mod').Foo` — the module
            // specifier is a string literal and the qualifier is in the
            // imported module's namespace, never a body-local. Only the
            // type arguments can reference outer names.
            if let Some(args) = &t.type_arguments {
                for arg in &args.params {
                    walk(arg, out);
                }
            }
        }
        TSType::TSIndexedAccessType(t) => {
            walk(&t.object_type, out);
            walk(&t.index_type, out);
        }
        TSType::TSInferType(t) => {
            out.idents
                .insert(SmolStr::from(t.type_parameter.name.name.as_str()));
            if let Some(constraint) = &t.type_parameter.constraint {
                walk(constraint, out);
            }
            if let Some(default) = &t.type_parameter.default {
                walk(default, out);
            }
        }
        TSType::TSIntersectionType(t) => {
            for member in &t.types {
                walk(member, out);
            }
        }
        TSType::TSMappedType(t) => {
            out.idents.insert(SmolStr::from(t.key.name.as_str()));
            walk(&t.constraint, out);
            if let Some(name_type) = &t.name_type {
                walk(name_type, out);
            }
            if let Some(type_annotation) = &t.type_annotation {
                walk(type_annotation, out);
            }
        }
        TSType::TSNamedTupleMember(t) => {
            out.idents.insert(SmolStr::from(t.label.name.as_str()));
            walk_tuple_element(&t.element_type, out);
        }
        TSType::TSTemplateLiteralType(t) => {
            for member in &t.types {
                walk(member, out);
            }
        }
        TSType::TSTupleType(t) => {
            for el in &t.element_types {
                walk_tuple_element(el, out);
            }
        }
        TSType::TSTypeLiteral(t) => {
            for sig in &t.members {
                walk_signature(sig, out);
            }
        }
        TSType::TSTypeOperatorType(t) => {
            // `keyof typeof X` (parentheses unwrapped) — the precise
            // shape the declare-const stub can't approximate.
            if t.operator == TSTypeOperatorOperator::Keyof
                && let TSType::TSTypeQuery(q) = t.type_annotation.without_parenthesized()
                && let Some(root) = type_query_root(&q.expr_name)
            {
                out.keyof_typeof_refs.insert(root);
            }
            walk(&t.type_annotation, out);
        }
        TSType::TSTypePredicate(t) => {
            if let TSTypePredicateName::Identifier(id) = &t.parameter_name {
                out.idents.insert(SmolStr::from(id.name.as_str()));
            }
            if let Some(ta) = &t.type_annotation {
                walk_type_annotation(ta, out);
            }
        }
        TSType::TSTypeQuery(t) => {
            if let Some(root) = type_query_root(&t.expr_name) {
                out.idents.insert(root.clone());
                out.typeof_refs.insert(root);
            }
            if let Some(args) = &t.type_arguments {
                for arg in &args.params {
                    walk(arg, out);
                }
            }
        }
        TSType::TSTypeReference(t) => {
            if let Some(root) = entity_name_root(&t.type_name) {
                out.idents.insert(root);
            }
            if let Some(args) = &t.type_arguments {
                for arg in &args.params {
                    walk(arg, out);
                }
            }
        }
        TSType::TSUnionType(t) => {
            for member in &t.types {
                walk(member, out);
            }
        }
        TSType::TSParenthesizedType(t) => walk(&t.type_annotation, out),
        TSType::JSDocNullableType(t) => walk(&t.type_annotation, out),
        TSType::JSDocNonNullableType(t) => walk(&t.type_annotation, out),
    }
}

/// Walk one interface / type-literal member signature.
fn walk_signature(sig: &TSSignature<'_>, out: &mut TypeDeps) {
    match sig {
        TSSignature::TSIndexSignature(s) => {
            for param in &s.parameters {
                walk_type_annotation(&param.type_annotation, out);
            }
            walk_type_annotation(&s.type_annotation, out);
        }
        TSSignature::TSPropertySignature(s) => {
            collect_property_key(&s.key, out);
            if let Some(ta) = &s.type_annotation {
                walk_type_annotation(ta, out);
            }
        }
        TSSignature::TSCallSignatureDeclaration(s) => {
            walk_type_params(s.type_parameters.as_deref(), out);
            walk_params(&s.params, out);
            if let Some(ta) = &s.return_type {
                walk_type_annotation(ta, out);
            }
        }
        TSSignature::TSConstructSignatureDeclaration(s) => {
            walk_type_params(s.type_parameters.as_deref(), out);
            walk_params(&s.params, out);
            if let Some(ta) = &s.return_type {
                walk_type_annotation(ta, out);
            }
        }
        TSSignature::TSMethodSignature(s) => {
            collect_property_key(&s.key, out);
            walk_type_params(s.type_parameters.as_deref(), out);
            walk_params(&s.params, out);
            if let Some(ta) = &s.return_type {
                walk_type_annotation(ta, out);
            }
        }
    }
}

/// Walk a generic-parameter declaration list: each parameter's name,
/// constraint and default.
fn walk_type_params(decl: Option<&TSTypeParameterDeclaration<'_>>, out: &mut TypeDeps) {
    let Some(decl) = decl else { return };
    for param in &decl.params {
        out.idents.insert(SmolStr::from(param.name.name.as_str()));
        if let Some(constraint) = &param.constraint {
            walk(constraint, out);
        }
        if let Some(default) = &param.default {
            walk(default, out);
        }
    }
}

/// Walk a formal-parameter list (function / constructor / method
/// signatures): binding names plus annotated types.
fn walk_params(params: &FormalParameters<'_>, out: &mut TypeDeps) {
    for item in &params.items {
        collect_binding_names(&item.pattern, out);
        if let Some(ta) = &item.type_annotation {
            walk_type_annotation(ta, out);
        }
    }
    if let Some(rest) = &params.rest {
        collect_binding_names(&rest.rest.argument, out);
        if let Some(ta) = &rest.type_annotation {
            walk_type_annotation(ta, out);
        }
    }
}

fn walk_type_annotation(ta: &TSTypeAnnotation<'_>, out: &mut TypeDeps) {
    walk(&ta.type_annotation, out);
}

fn walk_tuple_element(el: &TSTupleElement<'_>, out: &mut TypeDeps) {
    match el {
        TSTupleElement::TSOptionalType(t) => walk(&t.type_annotation, out),
        TSTupleElement::TSRestType(t) => walk(&t.type_annotation, out),
        // All remaining variants are inherited `TSType` variants.
        _ => {
            if let Some(ty) = el.as_ts_type() {
                walk(ty, out);
            }
        }
    }
}

/// Walk an interface `extends` clause: the heritage expression's root
/// identifier and its type arguments.
fn walk_heritage(heritage: &TSInterfaceHeritage<'_>, out: &mut TypeDeps) {
    if let Some(root) = expr_root(&heritage.expression) {
        out.idents.insert(root);
    }
    if let Some(args) = &heritage.type_arguments {
        for arg in &args.params {
            walk(arg, out);
        }
    }
}

/// Collect static-identifier property/method keys. String, numeric and
/// computed keys produce no plain identifier (matching the old byte
/// scanner, which skipped string-literal and bracketed keys).
fn collect_property_key(key: &PropertyKey<'_>, out: &mut TypeDeps) {
    match key {
        PropertyKey::StaticIdentifier(id) => {
            out.idents.insert(SmolStr::from(id.name.as_str()));
        }
        PropertyKey::PrivateIdentifier(_) => {}
        // Computed key `[A]: T` — the bracketed expression references a
        // value name (the byte scanner collected it too). Collect its
        // root identifier so a `declare const A` stub is still emitted.
        _ => {
            if let Some(expr) = key.as_expression()
                && let Some(root) = expr_root(expr)
            {
                out.idents.insert(root);
            }
        }
    }
}

fn collect_binding_names(pat: &BindingPattern<'_>, out: &mut TypeDeps) {
    match pat {
        BindingPattern::BindingIdentifier(id) => {
            out.idents.insert(SmolStr::from(id.name.as_str()));
        }
        BindingPattern::ObjectPattern(o) => {
            for prop in &o.properties {
                collect_binding_names(&prop.value, out);
            }
            if let Some(rest) = &o.rest {
                collect_binding_names(&rest.argument, out);
            }
        }
        BindingPattern::ArrayPattern(a) => {
            for el in a.elements.iter().flatten() {
                collect_binding_names(el, out);
            }
            if let Some(rest) = &a.rest {
                collect_binding_names(&rest.argument, out);
            }
        }
        BindingPattern::AssignmentPattern(a) => collect_binding_names(&a.left, out),
    }
}

/// Left-most identifier of a (possibly qualified) type name —
/// `Foo.Bar.Baz` → `Foo`. Mirrors upstream `getEntityNameRoot`.
fn entity_name_root(tn: &TSTypeName<'_>) -> Option<SmolStr> {
    tn.get_identifier_reference()
        .map(|r| SmolStr::from(r.name.as_str()))
}

/// Root identifier of a `typeof` query's entity name. Returns `None`
/// for `typeof import('mod')` (no entity-name root).
fn type_query_root(expr: &TSTypeQueryExprName<'_>) -> Option<SmolStr> {
    expr.as_ts_type_name().and_then(entity_name_root)
}

/// Root identifier of a heritage expression — `Base` / `ns.Base` →
/// the left-most name.
fn expr_root(e: &Expression<'_>) -> Option<SmolStr> {
    match e {
        Expression::Identifier(id) => Some(SmolStr::from(id.name.as_str())),
        Expression::StaticMemberExpression(m) => expr_root(&m.object),
        _ => None,
    }
}
