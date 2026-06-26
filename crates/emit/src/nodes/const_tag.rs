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
    let body = trimmed;

    // Void every binding introduced by the pattern. Without this tsgo
    // fires TS6133 on `@const` tags whose binding isn't referenced
    // elsewhere in the enclosing block.
    let lhs = split_lhs(body);
    for name in collect_pattern_names(&lhs) {
        let _ = writeln!(buf, "{indent}void {name};");
    }
}

/// Extract the binding-pattern prefix of an `{@const}` body, discarding
/// the type annotation and the initializer.
///
/// Examples:
///   - `foo = 1` → `foo`
///   - `foo: Record<A, B> = {}` → `foo`
///   - `[a, { b }] = tuple` → `[a, { b }]`
///   - `{ a = 1, b } = obj` → `{ a = 1, b }`
fn split_lhs(body: &str) -> String {
    let bytes = body.as_bytes();
    let mut depth = 0i32;
    let mut end = bytes.len();
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'{' | b'[' | b'(' | b'<' => depth += 1,
            b'}' | b']' | b')' | b'>' if depth > 0 => depth -= 1,
            b'=' if depth == 0 => {
                end = i;
                break;
            }
            b':' if depth == 0 => {
                end = i;
                break;
            }
            _ => {}
        }
        i += 1;
    }
    body[..end].trim().to_string()
}

/// Collect every identifier introduced by a (possibly destructuring)
/// binding pattern on the LHS of an `{@const}` tag.
///
/// Examples:
///   - `foo` → [foo]
///   - `{ a, b: c, ...rest }` → [a, c, rest]
///   - `[a, { b }, ...rest]` → [a, b, rest]
fn collect_pattern_names(lhs: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let bytes = lhs.as_bytes();
    let mut i = 0usize;
    let mut after_colon = false;
    let mut at_binding_start = true;
    let mut depth = 0i32;

    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' | b'[' => {
                depth += 1;
                i += 1;
                at_binding_start = true;
                after_colon = false;
                continue;
            }
            b'}' | b']' => {
                depth -= 1;
                i += 1;
                after_colon = false;
                at_binding_start = false;
                continue;
            }
            b',' => {
                i += 1;
                at_binding_start = true;
                after_colon = false;
                continue;
            }
            b':' => {
                i += 1;
                after_colon = true;
                at_binding_start = false;
                continue;
            }
            b'=' if depth > 0 => {
                let mut paren_depth = 0i32;
                i += 1;
                while i < bytes.len() {
                    match bytes[i] {
                        b'(' | b'[' | b'{' => paren_depth += 1,
                        b')' | b']' | b'}' if paren_depth > 0 => paren_depth -= 1,
                        b',' | b'}' | b']' if paren_depth == 0 => break,
                        _ => {}
                    }
                    i += 1;
                }
                continue;
            }
            b'.' if i + 2 < bytes.len() && &bytes[i..i + 3] == b"..." => {
                i += 3;
                at_binding_start = true;
                after_colon = false;
                continue;
            }
            b if b.is_ascii_whitespace() => {
                i += 1;
                continue;
            }
            _ => {}
        }
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
                    i += 1;
                } else {
                    break;
                }
            }
            let is_top_level = depth == 0;
            let is_binding = if is_top_level {
                true
            } else {
                let mut j = i;
                while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                    j += 1;
                }
                let next = bytes.get(j).copied();
                if after_colon {
                    true
                } else if at_binding_start {
                    !matches!(next, Some(b':'))
                } else {
                    false
                }
            };
            if is_binding {
                out.push(lhs[start..i].to_string());
            }
            at_binding_start = false;
            after_colon = false;
            continue;
        }
        at_binding_start = false;
        after_colon = false;
        i += 1;
    }
    out
}
