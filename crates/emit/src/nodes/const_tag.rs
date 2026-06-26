//! `{@const}` declaration emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/ConstTag.ts`.

use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// If `interp` is an `{@const <pattern> = <expr>}` tag, emit it inline
/// as a real `const <pattern> = <expr>;` statement in the current
/// template-check scope.
///
/// Without inline emission, the `@const`-declared name lives only as a
/// top-of-function `let NAME: any = undefined;` stub. That works for
/// "does the name resolve?" checks but drops the expression's inferred
/// type. A pattern like
///
/// ```text
///     {@const featureType = persistentFeature.settings.type}
///     {#if featureType === 'persistent-comment'}
/// ```
///
/// needs `featureType` to carry the discriminant literal type so TS's
/// control-flow analysis narrows it inside the `{#if}`. Emitting
/// inline pins the type. The top-level `let NAME: any` stub stays in
/// place so forward references (rare but possible) still resolve; the
/// inline `const` shadows it inside the block.
pub(crate) fn emit_at_const_if_any(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    if interp.kind != svn_parser::InterpolationKind::AtConst {
        return;
    }
    emit_declaration(buf, source, interp, depth, "const");
}

/// Emit a Svelte 5 declaration tag (`{const …}` / `{let …}`) inline as a
/// real `const <decl>;` / `let <decl>;` statement in the current
/// template-check scope. The `let` form keeps the binding mutable so
/// later reassignments in the same scope type-check. Mirrors upstream
/// svelte2tsx's `htmlxtojsx_v2/nodes/DeclarationTag.ts`, which strips the
/// braces and leaves the `const`/`let` declaration verbatim.
pub(crate) fn emit_declaration_tag(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    let keyword = match interp.kind {
        svn_parser::InterpolationKind::DeclConst => "const",
        svn_parser::InterpolationKind::DeclLet => "let",
        _ => return,
    };
    emit_declaration(buf, source, interp, depth, keyword);
}

/// Shared core for `{@const}` and declaration-tag emission. Emits
/// `INDENT<keyword> <body>;` followed by a `void` of each introduced
/// binding (suppresses TS6133 on an otherwise-unused declaration).
fn emit_declaration(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
    keyword: &str,
) {
    let body_start = interp.expression_range.start as usize;
    let body_end = interp.expression_range.end as usize;
    let Some(body_raw) = source.get(body_start..body_end) else {
        return;
    };
    let trimmed = body_raw.trim();
    if trimmed.is_empty() {
        return;
    }
    let indent = "    ".repeat(depth);
    // The body is emitted verbatim (UNTRIMMED + full expression_range) via
    // a TOKEN-map entry so its byte range matches the source slice exactly
    // and diagnostics land on the precise source column. `append_with_source`
    // (token map) is used rather than `append_verbatim` (line map): the
    // latter records a mapping ONLY for multi-line bodies, so a SINGLE-LINE
    // `{@const x = undefinedRef}` would drop its in-expression tsgo error
    // (e.g. TS2304) entirely — an under-report vs upstream. Token mapping
    // also handles multi-line bodies correctly (byte offset is preserved by
    // the verbatim copy).
    buf.push_str(&indent);
    buf.push_str(keyword);
    buf.push(' ');
    buf.append_with_source(body_raw, interp.expression_range);
    buf.push_str(";\n");

    // Void every binding the pattern introduces — suppresses TS6133 on a
    // `{@const}` / declaration tag whose binding isn't read elsewhere in
    // the block. Names come from the oxc parse of the declaration
    // (`svn_analyze::extract_at_const_bindings`), not a byte scan: correct
    // for destructure patterns with delimiters inside string defaults,
    // type annotations, nested rest, etc. (Rule #1).
    for name in svn_analyze::extract_at_const_bindings(interp, source) {
        let _ = writeln!(buf, "{indent}void {name};");
    }
}
