//! `$:` reactive-statement rewrite.
//!
//! Svelte 4's three shapes, each rewritten to a Svelte-5-shaped form
//! the existing emit pipeline handles:
//!
//! | Shape | Example | Rewrite |
//! |---|---|---|
//! | Declaration | `$: b = count * 2` (b not yet declared) | `let b = $derived(count * 2)` |
//! | Re-assignment | `$: count += 1` (count declared earlier) | `count += 1;` (drop label) |
//! | Statement / block | `$: { … }`, `$: console.log(a)` | `{ $: … };` (plain block wrapper) |
//!
//! Why `$derived` and not `__sveltets_2_invalidate` (upstream's helper)?
//! Our shim is a Svelte-5 world. `$derived<T>(expr: T): T` is already
//! an ambient rune declaration. Mapping to it keeps the emit closer to
//! what users would write in Svelte 5 and avoids pulling in an
//! upstream-only helper. Type-level semantics are identical.
//!
//! Why the arrow wrapper `;() => { $: … };` (and the leading `;`)?
//!
//! A naive block wrapper (`{ $: … };`) runs at top-level execution
//! time in TypeScript's control-flow analysis. References inside
//! the block to reactive-declared names later in the script fire
//! TS2454 / TS2448 "used before being assigned / used before its
//! declaration" — TS follows the sequential order of statements.
//! An arrow body, by contrast, is NEVER invoked at type-check time,
//! so TDZ analysis doesn't apply to its contents. Upstream
//! svelte2tsx uses the same arrow trick in
//! `ImplicitTopLevelNames.handleReactiveStatement`.
//!
//! The leading `;` defeats an ASI hazard that initially pushed us
//! toward the block wrap: without the semi, a preceding line ending
//! in `)` — think `get(`k`).then((v) => {})` — followed by
//! `() => { $: set(…) }` parses as `…then(…)(() => {…})`, the
//! arrow getting consumed as an argument to the prior call. The
//! leading `;` forces the prior statement to terminate before the
//! arrow begins.
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
use std::fmt::Write;

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
    rewrite_with_touched_names(content, lang).0
}

/// Like `rewrite`, but ALSO returns the set of identifier names that
/// the rewrite TOUCHED on the LHS of a reactive-destructure or a
/// reactive-reassignment to an already-declared name. Callers can
/// feed these names to the downstream definite-assign pass so the
/// pre-existing `let X: T;` declarations (Svelte-4's bare-typed prop
/// pattern) get a `!` — the reactive assignment counts as "assigned"
/// at runtime, but from TS's perspective the declaration is an
/// uninitialized let + later branch assignment hidden inside an
/// uncalled arrow body. Without the `!`, references elsewhere in
/// the script fire TS2454 "used before being assigned".
pub fn rewrite_with_touched_names(content: &str, lang: ScriptLang) -> (String, Vec<SmolStr>) {
    if !content.contains("$:") {
        return (content.to_string(), Vec::new());
    }
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, content, lang);

    let declared_vars = collect_top_level_var_names(&parsed.program);

    let mut edits: Vec<Edit> = Vec::new();
    let mut hoisted_names: Vec<SmolStr> = Vec::new();
    let mut touched_names: Vec<SmolStr> = Vec::new();
    for stmt in &parsed.program.body {
        let Statement::LabeledStatement(labeled) = stmt else {
            continue;
        };
        if labeled.label.name.as_str() != "$" {
            continue;
        }
        collect_touched_names_for_statement(labeled, &declared_vars, &mut touched_names);
        let edit = classify_and_rewrite(labeled, content, &declared_vars, &mut hoisted_names);
        edits.push(edit);
    }

    if edits.is_empty() {
        return (content.to_string(), touched_names);
    }

    // Apply edits in reverse byte order so earlier positions don't
    // shift when later ones are replaced.
    edits.sort_by_key(|e| std::cmp::Reverse(e.start));
    let mut out = content.to_string();
    for edit in edits {
        out.replace_range(edit.start..edit.end, &edit.replacement);
    }
    let _ = hoisted_names;
    (out, touched_names)
}

/// Walk a single `$:` labeled statement; if the LHS identifies (or
/// destructures) ALREADY-declared names, those names need a `!`
/// assertion on their original declaration — the reactive assignment
/// is hidden inside an arrow body and TS's flow analysis can't see
/// it. Names NOT already declared aren't added here (they get a
/// fresh `let NAME = $derived(…)` at the same position).
fn collect_touched_names_for_statement(
    labeled: &LabeledStatement<'_>,
    declared: &HashSet<SmolStr>,
    out: &mut Vec<SmolStr>,
) {
    let Statement::ExpressionStatement(expr_stmt) = &labeled.body else {
        return;
    };
    let inner_expr = match &expr_stmt.expression {
        Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    let Expression::AssignmentExpression(assign) = inner_expr else {
        return;
    };
    if !matches!(assign.operator, oxc_ast::ast::AssignmentOperator::Assign) {
        return;
    }
    // Case A: simple identifier LHS, already declared → re-assignment.
    if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) = &assign.left {
        let name = SmolStr::from(id.name.as_str());
        if declared.contains(&name) && !out.iter().any(|n| n == &name) {
            out.push(name);
        }
        return;
    }
    // Case B: destructure LHS — every destructured name counts (the
    // names might be pre-declared via bare `let X: T;` or freshly
    // introduced. For pre-declared ones we need the `!`; for fresh
    // ones the rewriter itself emits `let {…} = expr` which already
    // initialises, so the `!` is harmless in either case).
    for name in collect_destructure_names(&assign.left) {
        if declared.contains(&name) && !out.iter().any(|n| n == &name) {
            out.push(name);
        }
    }
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
        match stmt {
            Statement::VariableDeclaration(decl) => {
                for d in &decl.declarations {
                    collect_binding_names(d, &mut out);
                }
            }
            // `export let X = …` / `export const X = …` — Svelte 4
            // prop declarations. Their `X` IS declared at module scope
            // after our script_split strips the export keyword, so a
            // subsequent `$: X = …` must be treated as re-assignment
            // (drop the label) rather than a fresh declaration.
            Statement::ExportNamedDeclaration(decl) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(vd)) =
                    &decl.declaration
                {
                    for d in &vd.declarations {
                        collect_binding_names(d, &mut out);
                    }
                }
            }
            _ => {}
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
    hoisted_names: &mut Vec<SmolStr>,
) -> Edit {
    let full_start = labeled.span.start as usize;
    let full_end = labeled.span.end as usize;

    // Case 1: body is `IDENT = expr` (as an expression statement).
    // Distinguish declaration (IDENT not yet declared) vs re-assignment.
    if let Statement::ExpressionStatement(expr_stmt) = &labeled.body {
        // Destructure LHS forms (`({a, b} = expr)`) parse as a
        // ParenthesizedExpression wrapping the assignment.
        let inner_expr = match &expr_stmt.expression {
            Expression::ParenthesizedExpression(p) => &p.expression,
            other => other,
        };
        if let Expression::AssignmentExpression(assign) = inner_expr {
            if matches!(assign.operator, oxc_ast::ast::AssignmentOperator::Assign) {
                if let oxc_ast::ast::AssignmentTarget::AssignmentTargetIdentifier(id) =
                    &assign.left
                {
                    let name = id.name.as_str();
                    let rhs_span = assign.right.span();
                    let rhs = &content[rhs_span.start as usize..rhs_span.end as usize];

                    // `$foo` identifier (starts with `$`) — in Svelte-4
                    // this is a store auto-subscribe alias. The emit
                    // crate forward-declares it as `let $foo!: …` at
                    // the top of the render function; a reactive
                    // assignment `$: $foo = expr` is a store `.set()`
                    // shorthand, not a fresh declaration. Emitting a
                    // second `let $foo = …` would fire TS2451
                    // "redeclare block-scoped variable". Treat as
                    // re-assignment.
                    if declared.contains(&SmolStr::from(name)) || name.starts_with('$') {
                        // Re-assignment: drop the `$:` label, keep the
                        // assignment statement as-is. Emit `NAME = EXPR;`.
                        return Edit {
                            start: full_start,
                            end: full_end,
                            replacement: format!("{name} = {rhs};"),
                        };
                    } else {
                        // Declaration: `$: NAME = EXPR` introduces a
                        // fresh binding. Rewrite to
                        // `let NAME = __svn_invalidate(() => EXPR)`.
                        //
                        // Why `__svn_invalidate(() => EXPR)` instead
                        // of `$derived(EXPR)`? The thunk wrap makes
                        // TS's flow analysis lazy: any identifier
                        // inside EXPR that's declared LATER in the
                        // script (e.g. a const-arrow helper) doesn't
                        // fire TS2448 "used before its declaration".
                        // The return type still flows out as the
                        // inferred `T` of the thunk, so template-
                        // side type-checking against NAME is
                        // unchanged. Mirrors upstream svelte2tsx's
                        // `__sveltets_2_invalidate` helper. `void
                        // NAME;` suppresses TS6133 when NAME is only
                        // used in the template.
                        let _ = hoisted_names;
                        return Edit {
                            start: full_start,
                            end: full_end,
                            replacement: format!(
                                "let {name} = __svn_invalidate(() => ({rhs})); void {name};"
                            ),
                        };
                    }
                }
                // Case 1b: destructuring LHS — `$: ({a, b} = expr)` or
                // `$: ([a, b] = expr)`. Svelte 4 auto-declares each
                // destructured name at module scope; without a
                // rewrite, TS sees the names as undeclared and every
                // template reference to `a` / `b` fires TS2304.
                //
                // Rewrite to `let { a, b } = expr;` (or `let [a, b]`)
                // which declares AND initialises in one step. If any
                // name is already declared elsewhere in the script,
                // fall through to Case 2 (block wrap) — we can't
                // safely emit a fresh `let` for already-bound names.
                let destructure_names =
                    collect_destructure_names(&assign.left);
                if !destructure_names.is_empty()
                    && destructure_names
                        .iter()
                        .all(|n| !declared.contains(n))
                {
                    let rhs_span = assign.right.span();
                    let rhs = &content[rhs_span.start as usize..rhs_span.end as usize];
                    let lhs_span = assign.left.span();
                    let lhs = &content[lhs_span.start as usize..lhs_span.end as usize];
                    // Strip an outer `(...)` wrap — the JS form
                    // `({a, b} = expr)` parenthesises the object pattern
                    // so it parses as an expression rather than a block
                    // statement. TS's `let` declaration doesn't want
                    // that outer parens.
                    let lhs_trimmed = lhs.trim();
                    let lhs_unwrap = if lhs_trimmed.starts_with('(') && lhs_trimmed.ends_with(')') {
                        lhs_trimmed[1..lhs_trimmed.len() - 1].trim()
                    } else {
                        lhs_trimmed
                    };
                    // Emit `void NAME;` for each destructured name
                    // so TS6133 doesn't fire when the name is used
                    // only in the template (separate function scope
                    // later in the emit). Hoisting to `let NAME!: any`
                    // at the top is DELIBERATELY skipped — see the
                    // matching branch for the simple-identifier case.
                    let _ = hoisted_names;
                    let voids: String = destructure_names
                        .iter()
                        .map(|n| format!(" void {n};"))
                        .collect();
                    return Edit {
                        start: full_start,
                        end: full_end,
                        replacement: format!(
                            "let {lhs_unwrap} = __svn_invalidate(() => ({rhs}));{voids}"
                        ),
                    };
                }
            }
        }
    }

    // Case 2: anything else — block, expression statement without
    // `IDENT = expr`, etc. Wrap in `;() => { $: ORIGINAL };` — the
    // arrow form matches upstream svelte2tsx's emit (see its
    // ImplicitTopLevelNames.handleReactiveStatement). Two key
    // properties:
    //
    // - The arrow body is NEVER invoked at runtime, so TypeScript's
    //   control-flow analysis doesn't apply TDZ checks to its
    //   contents. A `$: if (…actuallySubmitted…)` that references a
    //   reactive-declared name later in the script resolves cleanly
    //   because `actuallySubmitted` only has to exist in the
    //   enclosing scope, not be ASSIGNED before the arrow runs. A
    //   block wrap (`{ $: … }`) would run at top-level execution
    //   time and fire TS2454.
    //
    // - Leading `;` defeats the ASI trap — a preceding line ending
    //   in `)` (like `get(…).then((v) => {})`) would otherwise
    //   splice with the arrow into a call chain
    //   `…then(…)(() => {…})`. The semicolon forces the prior
    //   statement to terminate.
    let original = &content[full_start..full_end];
    Edit {
        start: full_start,
        end: full_end,
        replacement: format!(";() => {{ {original} }};"),
    }
}

/// Collect every identifier name introduced by a destructuring
/// assignment-target pattern. Handles object patterns `{a, b: renamed}`,
/// array patterns `[a, b]`, nested patterns, and rest elements.
/// Returns the names in declaration order. Used only to decide whether
/// a `$: ({...} = expr)` reactive statement can be safely rewritten to
/// a `let {...} = expr;` declaration.
fn collect_destructure_names(
    target: &oxc_ast::ast::AssignmentTarget<'_>,
) -> Vec<SmolStr> {
    use oxc_ast::ast::{AssignmentTarget, AssignmentTargetProperty};
    let mut out = Vec::new();
    match target {
        AssignmentTarget::ArrayAssignmentTarget(arr) => {
            for elt in &arr.elements {
                let Some(elt) = elt else { continue };
                collect_from_maybe_default(elt, &mut out);
            }
            if let Some(rest) = &arr.rest {
                if let AssignmentTarget::AssignmentTargetIdentifier(id) = &rest.target {
                    out.push(SmolStr::from(id.name.as_str()));
                }
            }
        }
        AssignmentTarget::ObjectAssignmentTarget(obj) => {
            for prop in &obj.properties {
                match prop {
                    AssignmentTargetProperty::AssignmentTargetPropertyIdentifier(p) => {
                        out.push(SmolStr::from(p.binding.name.as_str()));
                    }
                    AssignmentTargetProperty::AssignmentTargetPropertyProperty(p) => {
                        collect_from_maybe_default(&p.binding, &mut out);
                    }
                }
            }
            if let Some(rest) = &obj.rest {
                if let AssignmentTarget::AssignmentTargetIdentifier(id) = &rest.target {
                    out.push(SmolStr::from(id.name.as_str()));
                }
            }
        }
        _ => {}
    }
    out
}

fn collect_from_maybe_default(
    node: &oxc_ast::ast::AssignmentTargetMaybeDefault<'_>,
    out: &mut Vec<SmolStr>,
) {
    use oxc_ast::ast::{AssignmentTarget, AssignmentTargetMaybeDefault};
    let target = match node {
        AssignmentTargetMaybeDefault::AssignmentTargetWithDefault(d) => &d.binding,
        _ => match node.as_assignment_target() {
            Some(t) => t,
            None => return,
        },
    };
    match target {
        AssignmentTarget::AssignmentTargetIdentifier(id) => {
            out.push(SmolStr::from(id.name.as_str()));
        }
        AssignmentTarget::ArrayAssignmentTarget(_)
        | AssignmentTarget::ObjectAssignmentTarget(_) => {
            out.extend(collect_destructure_names(target));
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        rewrite(src, ScriptLang::Ts)
    }

    #[test]
    fn declaration_form_becomes_invalidate() {
        let src = "$: b = count * 2;";
        assert_eq!(
            ts(src),
            "let b = __svn_invalidate(() => (count * 2)); void b;"
        );
    }

    #[test]
    fn declaration_form_without_semicolon() {
        let src = "$: b = count * 2";
        assert_eq!(
            ts(src),
            "let b = __svn_invalidate(() => (count * 2)); void b;"
        );
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
            got.starts_with(";() => { $: console.log(count); };"),
            "expression statement wrapped: {got:?}",
        );
    }

    #[test]
    fn block_wrapped_in_arrow() {
        let src = "$: { a = b; c = d; }";
        let got = ts(src);
        assert!(
            got.starts_with(";() => { $: { a = b; c = d; } };"),
            "block wrapped: {got:?}",
        );
    }

    #[test]
    fn wrap_survives_call_chain_preceding_line() {
        // A preceding `get(…).then(…)` followed by `() => { $: … }`
        // parses as `…then(…)(() => {…})` when no semi intervenes —
        // the arrow gets consumed as a call argument. Leading `;`
        // on the wrap forces the prior statement to terminate.
        let src = "get(`k`).then((v) => {})\n$: set(`k`, v)";
        let got = ts(src);
        assert!(
            got.contains(";() => { $: set(`k`, v) };"),
            "arrow wrap with leading semi: {got:?}",
        );
    }

    #[test]
    fn destructure_object_auto_declares() {
        // `$: ({a, b} = obj)` becomes `let {a, b} = __svn_invalidate(
        // () => obj);` — Svelte 4 auto-declares each destructured name
        // at module scope. The invalidate wrap defers flow analysis of
        // the RHS so forward references to later-declared consts don't
        // fire TS2448.
        let src = "$: ({ a, b } = question);";
        let got = ts(src);
        assert_eq!(
            got,
            "let { a, b } = __svn_invalidate(() => (question)); void a; void b;"
        );
    }

    #[test]
    fn destructure_with_renaming_auto_declares() {
        let src = "$: ({ a, b: renamed } = question);";
        let got = ts(src);
        assert_eq!(
            got,
            "let { a, b: renamed } = __svn_invalidate(() => (question)); void a; void renamed;"
        );
    }

    #[test]
    fn destructure_array_auto_declares() {
        let src = "$: ([x, y] = pair());";
        let got = ts(src);
        assert_eq!(
            got,
            "let [x, y] = __svn_invalidate(() => (pair())); void x; void y;"
        );
    }

    #[test]
    fn destructure_with_already_declared_name_falls_back_to_wrap() {
        // If any destructured name is already a module-scope declaration,
        // don't emit a fresh `let` (duplicate declaration). Fall back to
        // block wrap, which preserves the assignment semantics.
        let src = "let a = 0;\n$: ({ a, b } = question);";
        let got = ts(src);
        assert!(got.contains("let a = 0;"), "prior decl preserved: {got:?}");
        assert!(
            got.contains(";() => { $: ({ a, b } = question); };"),
            "arrow wrap fallback: {got:?}"
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
        assert!(got.contains("let a = __svn_invalidate(() => (1)); void a;"), "a invalidate: {got:?}");
        assert!(got.contains("let b = __svn_invalidate(() => (2)); void b;"), "b invalidate: {got:?}");
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
        assert!(
            got.contains("let y = __svn_invalidate(() => (x * 2)); void y;"),
            "y invalidate: {got:?}"
        );
    }

    #[test]
    fn preserves_surrounding_content() {
        let src = "const a = 1;\n\n$: b = a * 2;\n\nconst c = 3;\n";
        let got = ts(src);
        assert!(got.contains("const a = 1;"), "a preserved: {got:?}");
        assert!(got.contains("const c = 3;"), "c preserved: {got:?}");
        assert!(
            got.contains("let b = __svn_invalidate(() => (a * 2)); void b;"),
            "b invalidate: {got:?}"
        );
    }
}
