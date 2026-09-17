//! Type-dependency collection for the type-hoisting decision.
//!
//! Port of upstream svelte2tsx's `collectTypeDependencies`
//! (`language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/HoistableInterfaces.ts`):
//! walking a type, every type reference contributes the left-most
//! identifier of its name to the type dependencies, and every `typeof`
//! query contributes the left-most identifier of its expression to the
//! value dependencies. Nothing else — property keys, parameter names
//! and the like — is a dependency.
//!
//! The walk is over `oxc`'s TS-type AST (architecture rule #1). Every
//! `TSType` variant is matched explicitly so the compiler fails when
//! oxc adds one we haven't decided on.

use std::collections::HashSet;

use oxc_ast::ast::{
    FormalParameters, TSInterfaceDeclaration, TSSignature, TSTupleElement, TSType,
    TSTypeAliasDeclaration, TSTypeAnnotation, TSTypeName, TSTypeParameterDeclaration,
    TSTypeQueryExprName,
};
use smol_str::SmolStr;

/// What a type declaration depends on.
#[derive(Debug, Default, Clone)]
pub(crate) struct TypeDeps {
    /// Roots of the type references in the declaration, minus the
    /// declaration's own name and its own type parameters.
    pub type_refs: HashSet<SmolStr>,
    /// Roots of the `typeof` queries in the declaration.
    pub value_refs: HashSet<SmolStr>,
}

/// Dependencies of `type Foo<…> = …`: everything in the aliased type.
pub(crate) fn collect_alias_deps(decl: &TSTypeAliasDeclaration<'_>) -> TypeDeps {
    let mut out = TypeDeps::default();
    walk(&decl.type_annotation, &mut out);
    exclude_own_names(&mut out, &decl.id.name, decl.type_parameters.as_deref());
    out
}

/// Dependencies of `interface Foo<…> extends … { … }`. Upstream reads
/// only the interface's property types, its index signatures and its
/// `extends` clauses; method, call and construct signatures directly
/// on the interface contribute nothing.
pub(crate) fn collect_interface_deps(decl: &TSInterfaceDeclaration<'_>) -> TypeDeps {
    let mut out = TypeDeps::default();
    for sig in &decl.body.body {
        match sig {
            TSSignature::TSPropertySignature(s) => {
                if let Some(ta) = &s.type_annotation {
                    walk_type_annotation(ta, &mut out);
                }
            }
            TSSignature::TSIndexSignature(s) => {
                walk_type_annotation(&s.type_annotation, &mut out);
                walk_type_annotation(&s.parameter.type_annotation, &mut out);
            }
            TSSignature::TSCallSignatureDeclaration(_)
            | TSSignature::TSConstructSignatureDeclaration(_)
            | TSSignature::TSMethodSignature(_) => {}
        }
    }
    for heritage in &decl.extends {
        // Only a plain identifier names a dependency here; a dotted
        // `extends ns.Base` is an expression, not a type reference.
        if let TSTypeName::IdentifierReference(id) = &heritage.type_name {
            out.type_refs.insert(SmolStr::from(id.name.as_str()));
        }
        if let Some(args) = &heritage.type_arguments {
            for arg in &args.params {
                walk(arg, &mut out);
            }
        }
    }
    exclude_own_names(&mut out, &decl.id.name, decl.type_parameters.as_deref());
    out
}

/// Dependencies of a free-standing type, such as a `$props()`
/// annotation.
pub(crate) fn collect_type_node_deps(ty: &TSType<'_>) -> TypeDeps {
    let mut out = TypeDeps::default();
    walk(ty, &mut out);
    out
}

fn exclude_own_names(
    out: &mut TypeDeps,
    name: &str,
    params: Option<&TSTypeParameterDeclaration<'_>>,
) {
    out.type_refs.remove(name);
    for param in params.iter().flat_map(|p| p.params.iter()) {
        out.type_refs.remove(param.name.name.as_str());
    }
}

fn walk(ty: &TSType<'_>, out: &mut TypeDeps) {
    match ty {
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
            walk(&t.constraint, out);
            if let Some(name_type) = &t.name_type {
                walk(name_type, out);
            }
            if let Some(type_annotation) = &t.type_annotation {
                walk(type_annotation, out);
            }
        }
        TSType::TSNamedTupleMember(t) => walk_tuple_element(&t.element_type, out),
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
        TSType::TSTypeOperatorType(t) => walk(&t.type_annotation, out),
        TSType::TSTypePredicate(t) => {
            if let Some(ta) = &t.type_annotation {
                walk_type_annotation(ta, out);
            }
        }
        TSType::TSTypeQuery(t) => {
            if let Some(root) = t.expr_name.as_ts_type_name().map(entity_name_root) {
                out.value_refs.insert(root);
            }
            if let TSTypeQueryExprName::TSImportType(import) = &t.expr_name
                && let Some(args) = &import.type_arguments
            {
                for arg in &args.params {
                    walk(arg, out);
                }
            }
            if let Some(args) = &t.type_arguments {
                for arg in &args.params {
                    walk(arg, out);
                }
            }
        }
        TSType::TSTypeReference(t) => {
            out.type_refs.insert(entity_name_root(&t.type_name));
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

fn walk_signature(sig: &TSSignature<'_>, out: &mut TypeDeps) {
    match sig {
        TSSignature::TSIndexSignature(s) => {
            walk_type_annotation(&s.parameter.type_annotation, out);
            walk_type_annotation(&s.type_annotation, out);
        }
        TSSignature::TSPropertySignature(s) => {
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
            walk_type_params(s.type_parameters.as_deref(), out);
            walk_params(&s.params, out);
            if let Some(ta) = &s.return_type {
                walk_type_annotation(ta, out);
            }
        }
    }
}

fn walk_type_params(decl: Option<&TSTypeParameterDeclaration<'_>>, out: &mut TypeDeps) {
    for param in decl.iter().flat_map(|d| d.params.iter()) {
        if let Some(constraint) = &param.constraint {
            walk(constraint, out);
        }
        if let Some(default) = &param.default {
            walk(default, out);
        }
    }
}

fn walk_params(params: &FormalParameters<'_>, out: &mut TypeDeps) {
    for item in &params.items {
        if let Some(ta) = &item.type_annotation {
            walk_type_annotation(ta, out);
        }
    }
    if let Some(rest) = &params.rest
        && let Some(ta) = &rest.type_annotation
    {
        walk_type_annotation(ta, out);
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

/// Left-most identifier of a (possibly qualified) type name —
/// `Foo.Bar.Baz` → `Foo`; `this.x` → `this`.
pub(crate) fn entity_name_root(tn: &TSTypeName<'_>) -> SmolStr {
    match tn {
        TSTypeName::IdentifierReference(id) => SmolStr::from(id.name.as_str()),
        TSTypeName::QualifiedName(q) => entity_name_root(&q.left),
        TSTypeName::ThisExpression(_) => SmolStr::new_static("this"),
    }
}
