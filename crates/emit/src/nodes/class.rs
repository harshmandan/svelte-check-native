//! `class:foo={cond}` / `class:foo` (shorthand) directive emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Class.ts`.
//!
//! Each class directive becomes a bare expression statement inside the
//! element's scoped block, type-checking the reference without
//! constraining the attribute slot.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Emit a `class:NAME={cond}` / `class:NAME` (shorthand) directive as a
/// bare expression statement.
///
/// - With value: `(cond);` — TS type-checks the expression in scope.
/// - Without value (shorthand): `(NAME);` — same, but the identifier
///   is the directive name.
pub(crate) fn emit_class_directive(
    buf: &mut EmitBuffer,
    source: &str,
    d: &svn_parser::Directive,
    indent: &str,
) {
    match &d.value {
        Some(svn_parser::DirectiveValue::Expression {
            expression_range, ..
        }) => {
            let Some(expr) =
                source.get(expression_range.start as usize..expression_range.end as usize)
            else {
                return;
            };
            let trimmed = expr.trim();
            if trimmed.is_empty() {
                return;
            }
            let leading_ws = (expr.len() - expr.trim_start().len()) as u32;
            let start = expression_range.start + leading_ws;
            let end = start + trimmed.len() as u32;
            let _ = write!(buf, "{indent}(");
            buf.append_with_source(trimmed, svn_core::Range::new(start, end));
            let _ = writeln!(buf, ");");
        }
        _ => {
            // Shorthand `class:foo` — type-check `foo` as an
            // identifier reference.
            // svelte's parser gives the shorthand an identifier at the
            // name, so a missing name is reported there.
            let name_start = d.range.start + d.kind.prefix_len_with_colon();
            let name_range = svn_core::Range::new(name_start, name_start + d.name.len() as u32);
            let _ = write!(buf, "{indent}(");
            buf.append_with_source(d.name.as_str(), name_range);
            let _ = writeln!(buf, ");");
        }
    }
}
