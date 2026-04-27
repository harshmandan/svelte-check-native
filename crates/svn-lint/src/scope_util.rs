//! Pure AST/identifier helpers used by [`crate::scope`]. None of
//! these touch [`crate::scope::ScopeTree`] state — they're free
//! functions over `oxc_ast` expressions / patterns. Lifted out of
//! `scope.rs` so the main scope walker reads as visitor logic
//! rather than visitor logic + a bag of micro-helpers.

use oxc_ast::ast::{
    BindingPattern, BindingPatternKind, Expression, ForStatementInit, PropertyKey, Statement,
};
use oxc_span::GetSpan;

/// Flatten every binding identifier introduced by a destructure
/// pattern. Used by both the script-walker (to declare each
/// destructured name) and the export-let promotion pass (to gather
/// names from `let { a, b } = …;` form).
pub(crate) fn idents_in_pattern(pat: &BindingPattern<'_>) -> Vec<String> {
    let mut out = Vec::new();
    fn go(pat: &BindingPattern<'_>, out: &mut Vec<String>) {
        match &pat.kind {
            BindingPatternKind::BindingIdentifier(id) => out.push(id.name.to_string()),
            BindingPatternKind::ObjectPattern(op) => {
                for prop in &op.properties {
                    go(&prop.value, out);
                }
                if let Some(rest) = &op.rest {
                    go(&rest.argument, out);
                }
            }
            BindingPatternKind::ArrayPattern(ap) => {
                for p in ap.elements.iter().flatten() {
                    go(p, out);
                }
                if let Some(rest) = &ap.rest {
                    go(&rest.argument, out);
                }
            }
            BindingPatternKind::AssignmentPattern(ap) => go(&ap.left, out),
        }
    }
    go(pat, &mut out);
    out
}

/// Strip the leading identifier off an arbitrary string. `None`
/// when the first character isn't a valid identifier-start.
pub(crate) fn extract_base_ident(s: &str) -> Option<&str> {
    let mut end = 0;
    for (i, c) in s.char_indices() {
        if i == 0 && !(c.is_ascii_alphabetic() || c == '_' || c == '$') {
            return None;
        }
        if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    if end == 0 { None } else { Some(&s[..end]) }
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

/// Extract the `span.start` of an arbitrary `Statement` — oxc doesn't
/// expose a single uniform `span()` method, so we destructure.
pub(crate) fn statement_span_start(stmt: &Statement<'_>) -> Option<u32> {
    Some(stmt.span().start)
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
