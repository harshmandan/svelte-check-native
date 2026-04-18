//! `$:` reactive-statement rewrite.
//!
//! Svelte 4's three shapes, each rewritten to a Svelte-5-shaped form
//! the existing emit pipeline handles:
//!
//! | Shape | Example | Rewrite |
//! |---|---|---|
//! | Declaration | `$: b = count * 2` (b not yet declared) | `let b = $derived(count * 2)` |
//! | Re-assignment | `$: count += 1` (count declared earlier) | `count += 1;` (drop label) |
//! | Statement / block | `$: { … }`, `$: console.log(a)` | `() => { $: … }` (upstream's never-called arrow trick) |
//!
//! Why `$derived` and not `__sveltets_2_invalidate` (upstream's helper)?
//! Our shim is a Svelte-5 world. `$derived<T>(expr: T): T` is already
//! an ambient rune declaration. Mapping to it keeps the emit closer to
//! what users would write in Svelte 5 and avoids pulling in an
//! upstream-only helper. Type-level semantics are identical.
//!
//! Why the `() => { $: … }` wrapper for statement/block? `$: X` is a
//! label-prefixed statement in JavaScript (where `$` is the label
//! name). TypeScript type-checks the body but never invokes the arrow,
//! so runtime semantics are irrelevant. Upstream svelte2tsx uses this
//! trick; we match it verbatim.
//!
//! Scope of rewrites: **only top-level `$:` statements** in the instance
//! script body. Labels inside nested functions, classes, or blocks are
//! ordinary JS labels and left untouched.
//!
//! Position preservation: insertions happen in reverse byte-position
//! order so earlier spans aren't shifted by later rewrites. Within-line
//! column positions in the rewritten area drift; line numbers don't.
//! Tsgo diagnostics on rewritten code almost never fire (the rewrite
//! produces valid Svelte-5 shapes by construction), so column
//! precision loss is acceptable.

use std::collections::HashSet;

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    BindingPatternKind, Expression, LabeledStatement, Statement, VariableDeclarator,
};
use oxc_span::GetSpan;
use smol_str::SmolStr;
use svn_parser::{ScriptLang, parse_script_body};

/// Rewrite the Svelte-4 `$: ...` forms in `content` and return the
/// resulting source text. Cheap early-out: if the source contains no
/// `$:` literal substring at all (the common case on pure Svelte 5
/// components), returns `content.to_string()` without a parse.
pub fn rewrite(content: &str, lang: ScriptLang) -> String {
    if !content.contains("$:") {
        return content.to_string();
    }
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, content, lang);

    let declared_vars = collect_top_level_var_names(&parsed.program);

    let mut edits: Vec<Edit> = Vec::new();
    for stmt in &parsed.program.body {
        let Statement::LabeledStatement(labeled) = stmt else {
            continue;
        };
        if labeled.label.name.as_str() != "$" {
            continue;
        }
        let edit = classify_and_rewrite(labeled, content, &declared_vars);
        edits.push(edit);
    }

    if edits.is_empty() {
        return content.to_string();
    }

    // Apply edits in reverse byte order so earlier positions don't
    // shift when later ones are replaced.
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut out = content.to_string();
    for edit in edits {
        out.replace_range(edit.start..edit.end, &edit.replacement);
    }
    out
}

struct Edit {
    start: usize,
    end: usize,
    replacement: String,
}

/// Walk top-level `let`/`const`/`var` declarators and collect simple
/// identifier names. Used to tell a `$: x = expr` declaration from a
/// `$: x = expr` re-assignment.
fn collect_top_level_var_names(program: &oxc_ast::ast::Program<'_>) -> HashSet<SmolStr> {
    let mut out = HashSet::new();
    for stmt in &program.body {
        if let Statement::VariableDeclaration(decl) = stmt {
            for d in &decl.declarations {
                collect_binding_names(d, &mut out);
            }
        }
    }
    out
}

fn collect_binding_names(declarator: &VariableDeclarator<'_>, out: &mut HashSet<SmolStr>) {
    collect_from_pattern(&declarator.id.kind, out);
}

fn collect_from_pattern(pat: &BindingPatternKind<'_>, out: &mut HashSet<SmolStr>) {
    match pat {
        BindingPatternKind::BindingIdentifier(id) => {
            out.insert(SmolStr::from(id.name.as_str()));
        }
        BindingPatternKind::ObjectPattern(obj) => {
            for p in &obj.properties {
                collect_from_pattern(&p.value.kind, out);
            }
            if let Some(rest) = &obj.rest {
                collect_from_pattern(&rest.argument.kind, out);
            }
        }
        BindingPatternKind::ArrayPattern(arr) => {
            for el in arr.elements.iter().flatten() {
                collect_from_pattern(&el.kind, out);
            }
        }
        BindingPatternKind::AssignmentPattern(asn) => {
            collect_from_pattern(&asn.left.kind, out);
        }
    }
}

fn classify_and_rewrite(
    labeled: &LabeledStatement<'_>,
    content: &str,
    declared: &HashSet<SmolStr>,
) -> Edit {
    let full_start = labeled.span.start as usize;
    let full_end = labeled.span.end as usize;

    // Case 1: body is `IDENT = expr` (as an expression statement).
    // Distinguish declaration (IDENT not yet declared) vs re-assignment.
    if let Statement::ExpressionStatement(expr_stmt) = &labeled.body {
        if let Expression::AssignmentExpression(assign) = &expr_stmt.expression {
            if matches!(assign.operator, oxc_ast::ast::AssignmentOperator::Assign) {
                if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) =
                    &assign.left
                {
                    let name = id.name.as_str();
                    let rhs_span = assign.right.span();
                    let rhs = &content[rhs_span.start as usize..rhs_span.end as usize];

                    if declared.contains(&SmolStr::from(name)) {
                        // Re-assignment: drop the `$:` label, keep the
                        // assignment statement as-is. Emit `NAME = EXPR;`.
                        return Edit {
                            start: full_start,
                            end: full_end,
                            replacement: format!("{name} = {rhs};"),
                        };
                    } else {
                        // Declaration: `$: NAME = EXPR` introduces a
                        // fresh binding typed by EXPR. Rewrite to a
                        // `$derived` call so tsgo sees the same type
                        // flow without the label syntax.
                        return Edit {
                            start: full_start,
                            end: full_end,
                            replacement: format!("let {name} = $derived({rhs});"),
                        };
                    }
                }
            }
        }
    }

    // Case 2: anything else — block, expression statement without
    // `IDENT = expr`, etc. Wrap in `() => { $: ORIGINAL }` so TS
    // type-checks the body without parsing complaints about the label
    // at module scope.
    let original = &content[full_start..full_end];
    Edit {
        start: full_start,
        end: full_end,
        replacement: format!("() => {{ {original} }};"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        rewrite(src, ScriptLang::Ts)
    }

    #[test]
    fn declaration_form_becomes_derived() {
        let src = "$: b = count * 2;";
        assert_eq!(ts(src), "let b = $derived(count * 2);");
    }

    #[test]
    fn declaration_form_without_semicolon() {
        let src = "$: b = count * 2";
        assert_eq!(ts(src), "let b = $derived(count * 2);");
    }

    #[test]
    fn reassignment_drops_label() {
        // `count` declared earlier → `$: count = ...` is a re-assignment.
        let src = "let count = 0;\n$: count = count + 1;";
        let got = ts(src);
        assert!(
            got.contains("let count = 0;") && got.contains("count = count + 1;"),
            "both statements preserved: {got:?}",
        );
        assert!(
            !got.contains("$:"),
            "the `$:` label must be stripped: {got:?}",
        );
    }

    #[test]
    fn expression_statement_wrapped_in_arrow() {
        let src = "$: console.log(count);";
        let got = ts(src);
        assert!(
            got.starts_with("() => { $: console.log(count); };"),
            "expression statement wrapped: {got:?}",
        );
    }

    #[test]
    fn block_wrapped_in_arrow() {
        let src = "$: { a = b; c = d; }";
        let got = ts(src);
        assert!(
            got.starts_with("() => { $: { a = b; c = d; } };"),
            "block wrapped: {got:?}",
        );
    }

    #[test]
    fn non_dollar_label_untouched() {
        // `someLabel: for(...)` is a normal JS label, not a Svelte rune.
        let src = "someLabel: for (let i = 0; i < 10; i++) {}";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn nested_dollar_label_untouched() {
        // `$:` inside a function body isn't the top-level Svelte
        // reactive label — leave it alone. (JS label with name `$`.)
        let src = "function inner() { $: innerLabel; }";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn early_out_when_no_dollar_colon_in_source() {
        // Common case on pure Svelte 5 — source must pass through
        // unchanged without any parse.
        let src = "let a = 1; let b = 2;";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn multiple_reactive_declarations() {
        let src = "$: a = 1;\n$: b = 2;";
        let got = ts(src);
        assert!(got.contains("let a = $derived(1);"), "a derived: {got:?}");
        assert!(got.contains("let b = $derived(2);"), "b derived: {got:?}");
    }

    #[test]
    fn mixed_decl_and_reassign() {
        // `x` declared earlier → reassign. `y` not declared → derived.
        let src = "let x = 0;\n$: x = x + 1;\n$: y = x * 2;";
        let got = ts(src);
        assert!(
            got.contains("x = x + 1;") && !got.contains("$: x ="),
            "x reassignment: {got:?}",
        );
        assert!(got.contains("let y = $derived(x * 2);"), "y derived: {got:?}");
    }

    #[test]
    fn preserves_surrounding_content() {
        let src = "const a = 1;\n\n$: b = a * 2;\n\nconst c = 3;\n";
        let got = ts(src);
        assert!(got.contains("const a = 1;"), "a preserved: {got:?}");
        assert!(got.contains("const c = 3;"), "c preserved: {got:?}");
        assert!(got.contains("let b = $derived(a * 2);"), "b derived: {got:?}");
    }
}
