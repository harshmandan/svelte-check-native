//! `{#if}` / `{:else if}` block support.
//!
//! The actual `if`/`else if`/`else` branch dispatch lives in
//! `lib.rs::emit_template_node` (it has to interleave with the rest of
//! the dispatcher, since `{:else if}` is structurally a flat list of
//! arms rather than a nested tree). This module owns the support
//! helpers tsgo needs to type-check the condition expressions:
//!
//! - [`emit_condition_ref_marker`] — the `void [chain, …];` pacifier
//!   for TS2774 ("non-nullable function not invoked"), emitted at the
//!   top of each truthy-narrowed branch body.
//! - [`extract_property_chains`] — AST walker that collects the
//!   identifier / member-access chains referenced inside a condition
//!   expression.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/IfElseBlock.ts`.

use std::collections::HashSet;

use crate::emit_buffer::EmitBuffer;

/// Emit a `void [<access>, …];` statement listing every identifier /
/// property-access chain referenced inside the condition expression at
/// `range`. A no-op at runtime but a required pacifier for tsgo's
/// TS2774 check, which flags non-nullable-function operands of a
/// conditional `&&`/`||` chain unless the same symbol appears as a
/// value reference inside the enclosing block body.
///
/// Identifier-chain references are narrowing-neutral and still satisfy
/// the check: `isSymbolUsedInConditionBody` walks identifiers in the
/// body and matches on property-access chains with the same symbol.
/// We extract chains rather than whole logical operands so re-emitting
/// a comparison like `displayMode !== 'full'` inside a block that
/// already narrowed `displayMode` to exclude `'full'` doesn't fire
/// TS2367 ("this comparison appears to be unintentional").
///
/// We skip negated (`!(…)`) conditions entirely: TS2774 doesn't fire
/// reliably when the condition lives behind `!`, and emitting a
/// property-access marker inside the negated branch can surface type
/// errors (TS18047 / TS2339) that the user's inverted narrowing would
/// normally make unreachable.
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
///
/// Chain suffixes (`.prop`, `?.prop`) are included so `ctx.GhostButton`
/// emits as a single ref. Computed-member subscripts (`foo[key]`) and
/// call-argument lists (`f(x)`) are NOT swallowed: only the callee /
/// object portion contributes, matching the existing template-refs
/// pass convention.
///
/// Function bodies (arrow parameters, block expressions) are skipped
/// structurally — the AST walker simply doesn't recurse into them — so
/// inner-scope bindings never leak into the marker.
///
/// Keywords and the Svelte auto-subscribe `$ident` form are filtered
/// out post-walk.
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
