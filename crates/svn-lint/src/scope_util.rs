//! Pure AST/identifier helpers used by [`crate::scope`]. None of
//! these touch [`crate::scope::ScopeTree`] state — they're free
//! functions over `oxc_ast` expressions / patterns. Lifted out of
//! `scope.rs` so the main scope walker reads as visitor logic
//! rather than visitor logic + a bag of micro-helpers.

use oxc_ast::ast::{BindingPattern, Expression, ForStatementInit, PropertyKey};

/// Flatten every binding identifier introduced by a destructure
/// pattern. Used by both the script-walker (to declare each
/// destructured name) and the export-let promotion pass (to gather
/// names from `let { a, b } = …;` form).
pub(crate) fn idents_in_pattern<'a>(pat: &'a BindingPattern<'_>) -> Vec<&'a str> {
    binding_idents_in_pattern(pat)
        .into_iter()
        .map(|id| id.name.as_str())
        .collect()
}

/// The identifiers a binding pattern declares, in source order.
pub(crate) fn binding_idents_in_pattern<'a, 'b>(
    pat: &'a BindingPattern<'b>,
) -> Vec<&'a oxc_ast::ast::BindingIdentifier<'b>> {
    let mut out = Vec::new();
    fn go<'a, 'b>(
        pat: &'a BindingPattern<'b>,
        out: &mut Vec<&'a oxc_ast::ast::BindingIdentifier<'b>>,
    ) {
        match pat {
            BindingPattern::BindingIdentifier(id) => out.push(id),
            BindingPattern::ObjectPattern(op) => {
                for prop in &op.properties {
                    go(&prop.value, out);
                }
                if let Some(rest) = &op.rest {
                    go(&rest.argument, out);
                }
            }
            BindingPattern::ArrayPattern(ap) => {
                for p in ap.elements.iter().flatten() {
                    go(p, out);
                }
                if let Some(rest) = &ap.rest {
                    go(&rest.argument, out);
                }
            }
            BindingPattern::AssignmentPattern(ap) => go(&ap.left, out),
        }
    }
    go(pat, &mut out);
    out
}

/// The leftmost identifier of an expression given as source text
/// (`rest[0]` → `rest`, `/* c */ rést.b` → `rést`), or `None` when the
/// text isn't an identifier / member chain. Parses the slice, so
/// comments and non-ASCII names are handled like everything else.
pub(crate) fn base_identifier_of_text(slice: &str) -> Option<String> {
    use oxc_ast::ast::Statement;

    let wrapped = format!("({slice});");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
        return None;
    };
    let mut expr = &stmt.expression;
    while let Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }
    base_identifier(expr).map(|(name, _, _)| name.to_string())
}

/// The identifier an expression given as source text consists of —
/// once parentheses and TypeScript wrappers are removed, as the
/// compiler sees it — with its byte span within `slice`.
pub(crate) fn identifier_of_text(slice: &str) -> Option<(String, u32, u32)> {
    use oxc_ast::ast::Statement;

    let wrapped = format!("({slice});");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
        return None;
    };
    match unwrap_ts_wrappers(&stmt.expression) {
        // The `(` prefix shifts every span by one.
        Expression::Identifier(id) => Some((
            id.name.to_string(),
            id.span.start.checked_sub(1)?,
            id.span.end.checked_sub(1)?,
        )),
        _ => None,
    }
}

/// Walk to the leftmost identifier of a member-chain expression.
/// Returns `(name, span_start, span_end)` so callers can record
/// the binding-reference range. Drops past `Identifier` /
/// `StaticMemberExpression` / `ComputedMemberExpression` only —
/// any other node form returns `None`.
pub(crate) fn base_identifier<'a>(e: &'a Expression<'_>) -> Option<(&'a str, u32, u32)> {
    match e {
        Expression::Identifier(id) => Some((id.name.as_str(), id.span.start, id.span.end)),
        Expression::StaticMemberExpression(m) => base_identifier(&m.object),
        Expression::ComputedMemberExpression(m) => base_identifier(&m.object),
        _ => None,
    }
}

/// Peel off TS-only expression wrappers so rune-call detection sees
/// the `$state(…)` call inside `$state<T>() as unknown as X` etc.
/// Mirrors upstream's `remove_typescript_nodes` phase.
pub(crate) fn unwrap_ts_wrappers<'e, 'a>(expr: &'e Expression<'a>) -> &'e Expression<'a> {
    let mut cur = expr;
    loop {
        match cur {
            Expression::TSAsExpression(t) => cur = &t.expression,
            Expression::TSSatisfiesExpression(t) => cur = &t.expression,
            Expression::TSNonNullExpression(t) => cur = &t.expression,
            Expression::TSTypeAssertion(t) => cur = &t.expression,
            Expression::TSInstantiationExpression(t) => cur = &t.expression,
            Expression::ParenthesizedExpression(p) => cur = &p.expression,
            _ => return cur,
        }
    }
}

/// Return the body of a `// …` line comment or `/* … */` block
/// comment. `None` for any other prefix (the caller already
/// stripped whitespace, so a non-comment slice is a programming
/// error).
pub(crate) fn strip_comment_delimiters(text: &str) -> Option<&str> {
    if let Some(rest) = text.strip_prefix("//") {
        Some(rest)
    } else if let Some(rest) = text.strip_prefix("/*") {
        Some(rest.trim_end_matches("*/"))
    } else {
        None
    }
}

pub(crate) fn expression_from_for_init<'a>(
    e: &'a ForStatementInit<'_>,
) -> Option<&'a Expression<'a>> {
    e.as_expression()
}

pub(crate) fn expression_from_default<'a>(
    e: &'a oxc_ast::ast::ExportDefaultDeclarationKind<'_>,
) -> Option<&'a Expression<'a>> {
    e.as_expression()
}

pub(crate) fn expression_from_property_key<'a>(
    k: &'a PropertyKey<'_>,
) -> Option<&'a Expression<'a>> {
    k.as_expression()
}
