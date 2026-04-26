//! `{expr}` interpolations, `{@const}` declarations, and the
//! `void [chain, …];` condition-ref marker emitted inside `{#if}` /
//! `{:else if}` arms (TS2774 pacifier).
//!
//! Mirrors upstream svelte2tsx's `htmlxtojsx_v2/nodes/MustacheTag.ts` +
//! `ConstTag.ts`, with our token-map plumbing layered in.

use std::collections::HashSet;
use std::fmt::Write;

use crate::emit_buffer::EmitBuffer;

/// Emit a `{expr}` interpolation as a bare-paren-call expression
/// statement (`(EXPR);`). Routes `{@const}` / `{@html}` / `{@render}` /
/// `{@debug}` through `emit_at_const_if_any`; only `@const` produces
/// structured output today.
pub(crate) fn emit_interpolation(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    if interp.kind != svn_parser::InterpolationKind::Expression {
        emit_at_const_if_any(buf, source, interp, depth);
        return;
    }
    let expr_start = interp.expression_range.start as usize;
    let expr_end = interp.expression_range.end as usize;
    let Some(expr_raw) = source.get(expr_start..expr_end) else {
        return;
    };
    let trimmed = expr_raw.trim();
    if trimmed.is_empty() {
        return;
    }
    // Shift the source range to point at the trimmed expression so the
    // TokenMapEntry's overlay span exactly matches the source bytes
    // tsgo would blame for a diagnostic.
    let leading_ws = expr_raw.len() - expr_raw.trim_start().len();
    let trimmed_source_start = interp.expression_range.start + leading_ws as u32;
    let trimmed_source_end = trimmed_source_start + trimmed.len() as u32;
    let indent = "    ".repeat(depth);
    buf.append_synthetic(&indent);
    buf.append_synthetic("(");
    buf.append_with_source(
        trimmed,
        svn_core::Range::new(trimmed_source_start, trimmed_source_end),
    );
    buf.append_synthetic(");\n");
}

/// If `interp` is an `{@const <pattern> = <expr>}` tag, emit it inline
/// as a real `const <pattern> = <expr>;` statement in the current
/// template-check scope.
///
/// Without inline emission, the `@const`-declared name lives only as a
/// top-of-function `let NAME: any = undefined;` stub. That works for
/// "does the name resolve?" checks but drops the expression's inferred
/// type. A pattern like
///     `{@const featureType = persistentFeature.settings.type}`
///     `{#if featureType === 'persistent-comment'}`
/// needs `featureType` to carry the discriminant literal type so TS's
/// control-flow analysis narrows it inside the `{#if}`. Emitting
/// inline pins the type. The top-level `let NAME: any` stub stays in
/// place so forward references (rare but possible) still resolve; the
/// inline `const` shadows it inside the block.
fn emit_at_const_if_any(
    buf: &mut EmitBuffer,
    source: &str,
    interp: &svn_parser::Interpolation,
    depth: usize,
) {
    if interp.kind != svn_parser::InterpolationKind::AtConst {
        return;
    }
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
    // The body is emitted via `append_verbatim` so diagnostics
    // landing inside a multi-line body map back to the source
    // line tsgo reported. Use the UNTRIMMED body + full
    // expression_range so count_newlines(text) matches
    // count_newlines(source_slice) — trim-dropped leading whitespace
    // would desync the entry's source mapping by one line.
    buf.push_str(&indent);
    buf.push_str("const ");
    buf.append_verbatim(body_raw, source, interp.expression_range);
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

/// Emit a `void [<access>, …];` statement listing every identifier /
/// property-access chain referenced inside the condition expression at
/// `range`. A no-op at runtime but a required pacifier for tsgo's
/// TS2774 check, which flags non-nullable-function operands of a
/// conditional `&&`/`||` chain unless the same symbol appears as a
/// value reference inside the enclosing block body.
pub(crate) fn emit_condition_ref_marker(
    buf: &mut EmitBuffer,
    source: &str,
    range: svn_core::Range,
    depth: usize,
) {
    let Some(cond_text) = source.get(range.start as usize..range.end as usize) else {
        return;
    };
    if cond_text.trim_start().starts_with('!') {
        return;
    }
    let chains = extract_property_chains(cond_text);
    if chains.is_empty() {
        return;
    }
    let indent = "    ".repeat(depth);
    buf.push_str(&indent);
    buf.push_str("void [");
    for (i, chain) in chains.iter().enumerate() {
        if i > 0 {
            buf.push_str(", ");
        }
        buf.push_str(chain);
    }
    buf.push_str("];\n");
}

/// Extract every top-level identifier-or-property-access chain from
/// `text`, deduplicated and in source order.
///
/// Uses oxc to parse `text` as an expression, then walks the AST
/// collecting `Identifier` and `*MemberExpression` chains at the top
/// level of the expression's logical / binary structure. Per CLAUDE.md
/// rule #1 — the byte walker version was fragile: string escapes inside
/// template literals, RegExp literals, `?.` after non-identifier, etc.
/// all needed hand-coded handling. The AST walker gets each of these
/// for free.
pub(crate) fn extract_property_chains(text: &str) -> Vec<String> {
    use oxc_ast::ast::{Expression, Statement};

    let alloc = oxc_allocator::Allocator::default();
    let src = format!("({text});");
    let parsed = svn_parser::parse_script_body(&alloc, &src, svn_parser::ScriptLang::Ts);
    if parsed.panicked {
        return Vec::new();
    }
    let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
        return Vec::new();
    };
    let mut expr = &stmt.expression;
    while let Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }

    let mut out: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    fn chain_text<'a>(expr: &Expression<'_>, src: &'a str) -> Option<&'a str> {
        use oxc_ast::ast::Expression::*;
        use oxc_span::GetSpan;
        match expr {
            Identifier(_) => Some(&src[expr.span().start as usize..expr.span().end as usize]),
            StaticMemberExpression(m) => {
                let _ = chain_text(&m.object, src)?;
                Some(&src[m.span.start as usize..m.span.end as usize])
            }
            ComputedMemberExpression(m) => {
                let _ = chain_text(&m.object, src)?;
                Some(&src[m.object.span().start as usize..m.object.span().end as usize])
            }
            PrivateFieldExpression(m) => {
                let _ = chain_text(&m.object, src)?;
                Some(&src[m.span.start as usize..m.span.end as usize])
            }
            ChainExpression(c) => match &c.expression {
                oxc_ast::ast::ChainElement::CallExpression(_) => None,
                oxc_ast::ast::ChainElement::TSNonNullExpression(n) => {
                    chain_text(&n.expression, src)
                }
                oxc_ast::ast::ChainElement::ComputedMemberExpression(m) => {
                    let _ = chain_text(&m.object, src)?;
                    Some(&src[m.object.span().start as usize..m.object.span().end as usize])
                }
                oxc_ast::ast::ChainElement::StaticMemberExpression(m) => {
                    let _ = chain_text(&m.object, src)?;
                    Some(&src[m.span.start as usize..m.span.end as usize])
                }
                oxc_ast::ast::ChainElement::PrivateFieldExpression(m) => {
                    let _ = chain_text(&m.object, src)?;
                    Some(&src[m.span.start as usize..m.span.end as usize])
                }
            },
            _ => None,
        }
    }

    fn walk(expr: &Expression<'_>, src: &str, out: &mut Vec<String>, seen: &mut HashSet<String>) {
        use oxc_ast::ast::Expression::*;

        if let Some(text) = chain_text(expr, src) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                let looks_like_keyword = !trimmed.contains('.') && is_keyword_or_special(trimmed);
                if !looks_like_keyword {
                    let key = trimmed.to_string();
                    if seen.insert(key.clone()) {
                        out.push(key);
                    }
                }
            }
            return;
        }

        match expr {
            LogicalExpression(e) => {
                walk(&e.left, src, out, seen);
                walk(&e.right, src, out, seen);
            }
            BinaryExpression(e) => {
                walk(&e.left, src, out, seen);
                walk(&e.right, src, out, seen);
            }
            ConditionalExpression(e) => {
                walk(&e.test, src, out, seen);
                walk(&e.consequent, src, out, seen);
                walk(&e.alternate, src, out, seen);
            }
            SequenceExpression(e) => {
                for ex in &e.expressions {
                    walk(ex, src, out, seen);
                }
            }
            UnaryExpression(e) => {
                walk(&e.argument, src, out, seen);
            }
            AwaitExpression(e) => {
                walk(&e.argument, src, out, seen);
            }
            YieldExpression(e) => {
                if let Some(arg) = &e.argument {
                    walk(arg, src, out, seen);
                }
            }
            UpdateExpression(e) => match &e.argument {
                oxc_ast::ast::SimpleAssignmentTarget::AssignmentTargetIdentifier(_)
                | oxc_ast::ast::SimpleAssignmentTarget::ComputedMemberExpression(_)
                | oxc_ast::ast::SimpleAssignmentTarget::StaticMemberExpression(_)
                | oxc_ast::ast::SimpleAssignmentTarget::PrivateFieldExpression(_)
                | oxc_ast::ast::SimpleAssignmentTarget::TSAsExpression(_)
                | oxc_ast::ast::SimpleAssignmentTarget::TSSatisfiesExpression(_)
                | oxc_ast::ast::SimpleAssignmentTarget::TSNonNullExpression(_)
                | oxc_ast::ast::SimpleAssignmentTarget::TSTypeAssertion(_)
                | oxc_ast::ast::SimpleAssignmentTarget::TSInstantiationExpression(_) => {}
            },
            ParenthesizedExpression(p) => {
                walk(&p.expression, src, out, seen);
            }
            CallExpression(c) => {
                walk(&c.callee, src, out, seen);
            }
            TSAsExpression(e) => walk(&e.expression, src, out, seen),
            TSSatisfiesExpression(e) => walk(&e.expression, src, out, seen),
            TSNonNullExpression(e) => walk(&e.expression, src, out, seen),
            TSTypeAssertion(e) => walk(&e.expression, src, out, seen),
            TSInstantiationExpression(e) => walk(&e.expression, src, out, seen),
            _ => {}
        }
    }

    walk(expr, &src, &mut out, &mut seen);
    out
}

/// Keywords and reserved identifiers that should never appear in a
/// ref-marker void-array. Mirrors the filter in `template_refs` so a
/// condition like `{#if typeof x === 'string'}` doesn't emit a stray
/// `typeof` reference.
fn is_keyword_or_special(s: &str) -> bool {
    matches!(
        s,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "void"
            | "typeof"
            | "new"
            | "instanceof"
            | "in"
            | "of"
            | "as"
            | "let"
            | "const"
            | "var"
            | "function"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "return"
            | "yield"
            | "await"
            | "async"
            | "delete"
            | "throw"
            | "try"
            | "catch"
            | "finally"
            | "switch"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "class"
            | "extends"
            | "super"
            | "import"
            | "export"
            | "from"
            | "satisfies"
    ) || s.starts_with('$')
}
