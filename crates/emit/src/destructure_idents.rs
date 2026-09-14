//! The names a template binding site introduces.
//!
//! Used by emit when it needs the names a `{#each items as <pat>}`,
//! `{#snippet name(<params>)}`, `{:then <pat>}`, or `{:catch <pat>}`
//! introduces — emit declares or voids each one inside the enclosing
//! scope so descendant template references resolve and unused-variable
//! reports land on the right names.
//!
//! Both helpers parse the slice with oxc and walk the resulting
//! binding pattern. Reading names out of the text used to invent
//! bindings from a quoted key, a string default, a string type or the
//! `>` of `=>`, and to declare a computed key as a binding.

use smol_str::SmolStr;
use svn_analyze::template_scope::collect_pattern_bindings;

/// Every identifier a destructuring pattern binds, in source order.
///
/// For `id`              → `["id"]`
/// For `[id, label]`     → `["id", "label"]`
/// For `[id, { label }]` → `["id", "label"]`
/// For `{ a: x, b }`     → `["x", "b"]` (only the local-name side of `key:value`)
///
/// Falls back to a single `__svn_each_unused` token when the slice
/// binds nothing, so the emitted `void` line stays valid.
pub(crate) fn pattern_binding_names(pattern: &str) -> Vec<SmolStr> {
    use oxc_ast::ast::Statement;

    let wrapped = format!("let {pattern} = 0;");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let mut out = Vec::new();
    if let Some(Statement::VariableDeclaration(decl)) = parsed.program.body.first()
        && let Some(d) = decl.declarations.first()
    {
        out.extend(
            collect_pattern_bindings(&d.id, 0)
                .bindings
                .into_iter()
                .map(|b| b.name),
        );
    }
    if out.is_empty() {
        out.push(SmolStr::new_static("__svn_each_unused"));
    }
    out
}

/// Every identifier a snippet parameter list binds, in source order.
/// Type annotations and default values are part of the list and are
/// skipped by the parse.
pub(crate) fn param_binding_names(params: &str) -> Vec<SmolStr> {
    use oxc_ast::ast::{Expression, Statement};

    let wrapped = format!("({params}) => 0;");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
        return Vec::new();
    };
    let Expression::ArrowFunctionExpression(arrow) = &stmt.expression else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for param in &arrow.params.items {
        out.extend(
            collect_pattern_bindings(&param.pattern, 0)
                .bindings
                .into_iter()
                .map(|b| b.name),
        );
    }
    if let Some(rest) = &arrow.params.rest {
        out.extend(
            collect_pattern_bindings(&rest.rest.argument, 0)
                .bindings
                .into_iter()
                .map(|b| b.name),
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::{param_binding_names, pattern_binding_names};

    fn pat(s: &str) -> Vec<String> {
        pattern_binding_names(s)
            .iter()
            .map(|n| n.to_string())
            .collect()
    }
    fn params(s: &str) -> Vec<String> {
        param_binding_names(s)
            .iter()
            .map(|n| n.to_string())
            .collect()
    }

    #[test]
    fn pattern_shapes() {
        assert_eq!(pat("id"), ["id"]);
        assert_eq!(pat("[id, { label }]"), ["id", "label"]);
        assert_eq!(pat("{ a: x, b }"), ["x", "b"]);
        assert_eq!(pat("{ 'my-key': v }"), ["v"]);
        assert_eq!(pat("{ a = 'p,qq' }"), ["a"]);
        assert_eq!(pat("{ [k]: v }"), ["v"]);
        assert_eq!(pat(""), ["__svn_each_unused"]);
    }

    #[test]
    fn param_shapes() {
        assert_eq!(params("a: 'x' | 'y,zed'"), ["a"]);
        assert_eq!(params("a: Map<() => void, string>, b = 2"), ["a", "b"]);
        assert_eq!(
            params("{ months, weekdays }, ...rest"),
            ["months", "weekdays", "rest"]
        );
        assert!(params("").is_empty());
    }
}
