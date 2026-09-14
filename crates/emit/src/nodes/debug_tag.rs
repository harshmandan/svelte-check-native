//! `{@debug a, b}` debug tag.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/DebugTag.ts`.
//!
//! Upstream emits each comma-separated identifier as a bare statement
//! (`{@debug a, b}` → `;a;b;`) so tsgo type-checks each identifier
//! against the surrounding scope.
//!
//! We split the body on top-level commas and emit one
//! `(IDENT);` per part, with a TokenMapEntry per part so TS2304
//! diagnostics land at the user's source position rather than the
//! `{@debug` keyword.

use crate::emit_buffer::EmitBuffer;

/// Emit `{@debug a, b, …}` as one bare `(IDENT);` per listed
/// expression so tsgo fires TS2304 on typo'd names.
///
/// The body is parsed as a parenthesised expression: a sequence yields
/// one part per element, anything else is a single part. Parsing (not
/// splitting on commas) is what keeps a comma inside a comment or a
/// string from producing a fragment that fails to parse — and a
/// parse failure in any file hides every type error in the workspace.
pub(crate) fn emit_debug_tag(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    use oxc_ast::ast::{Expression, Statement};
    use oxc_span::GetSpan;

    let expr_start = interp.expression_range.start as usize;
    let expr_end = interp.expression_range.end as usize;
    let Some(body_raw) = source.get(expr_start..expr_end) else {
        return;
    };
    if body_raw.trim().is_empty() {
        // `{@debug}` with no expressions — runtime "log every reactive
        // value" form. Nothing to type-check.
        return;
    }
    // A leading `(` keeps a body that starts with `{` from parsing as a
    // block; a trailing newline closes any line comment before the `)`.
    let wrapped = format!("({body_raw}\n);");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return;
    }
    let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
        return;
    };
    let mut expr = &stmt.expression;
    while let Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }
    let spans: Vec<oxc_span::Span> = match expr {
        Expression::SequenceExpression(seq) => seq.expressions.iter().map(|e| e.span()).collect(),
        other => vec![other.span()],
    };
    let indent = "    ".repeat(depth);
    for span in spans {
        // Subtract the one-byte `(` prefix to land in the user's source.
        let Some(part) = wrapped.get(span.start as usize..span.end as usize) else {
            continue;
        };
        let abs_start = interp.expression_range.start + span.start - 1;
        let abs_end = abs_start + part.len() as u32;
        buf.append_synthetic(&indent);
        buf.append_synthetic("(");
        buf.append_with_source(part, svn_core::Range::new(abs_start, abs_end));
        buf.append_synthetic(");\n");
    }
}
