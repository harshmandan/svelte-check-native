//! Reviewer follow-up #3b: rewrite untyped `createEventDispatcher()`
//! calls to `createEventDispatcher<__SvnCustomEvents<$$Events>>()`
//! when an `interface $$Events` (or `type $$Events`) is declared.
//!
//! Without the rewrite, `dispatch('name', detail)` calls inside the
//! component go un-checked: the dispatcher's signature is generic
//! over its `<T>` arg, but with no `<T>` supplied TS infers `{}` as
//! the detail-map and any first arg passes. Upstream svelte2tsx's
//! `ComponentEvents.ts:130` does the same rewrite.
//!
//! Mirrors the existing `state_nullish_rewrite` shape: walk the
//! parsed AST for top-level `const X = createEventDispatcher()` (with
//! NO type arguments), record byte-positions of insertion points,
//! splice in `<__SvnCustomEvents<$$Events>>` after the call's
//! callee identifier so the call becomes
//! `createEventDispatcher<__SvnCustomEvents<$$Events>>()`.
//!
//! Aliased imports (`import { createEventDispatcher as ced }`) are
//! resolved via `find_typed_dispatcher_local_names`-style ctor-locals
//! inference; non-Svelte imports / local functions named
//! `createEventDispatcher` are excluded by the same gate that
//! `find_dispatcher_local_names` / `find_typed_dispatcher_local_names`
//! use (only `from 'svelte'` counts).

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Expression, Statement};
use svn_parser::{ScriptLang, parse_script_body};

/// Walk top-level `const X = createEventDispatcher()` declarators and
/// return `content` with `<__SvnCustomEvents<$$Events>>` spliced in
/// after each untyped dispatcher call's callee identifier. When no
/// untyped dispatcher is found (or no `interface $$Events` is
/// declared, per the caller's `should_rewrite` gate), returns
/// `content` unchanged.
pub fn rewrite(content: &str, lang: ScriptLang) -> String {
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, content, lang);

    let ctor_locals = collect_ctor_locals(&parsed.program);
    if ctor_locals.is_empty() {
        return content.to_string();
    }

    let mut insertions: Vec<(usize, &'static str)> = Vec::new();
    // Round-13 follow-up #2: walk recursively so nested untyped
    // dispatchers (inside function bodies, control-flow blocks,
    // callback args) also get the typed-events rewrite. Pre-fix
    // only top-level VariableDeclarations were rewritten;
    // `function f() { const d = createEventDispatcher() }`'s `d`
    // dispatched calls then bypassed the typed-events check.
    for stmt in &parsed.program.body {
        collect_rewrite_insertions(stmt, &ctor_locals, &mut insertions);
    }

    if insertions.is_empty() {
        return content.to_string();
    }
    // Reverse-sort by position so later insertions don't shift
    // earlier ones.
    insertions.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
    let mut out = content.to_string();
    for (pos, text) in insertions {
        out.insert_str(pos, text);
    }
    out
}

/// Round-13 #2-rewrite: walk a statement and collect the byte
/// position of every untyped `<ctor-local>()` call's callee end.
/// Recurses through Function/Block/If/For/While/Switch/Try/
/// LabeledStatement bodies + arrow/function expression bodies
/// attached as VarDecl initializers / IIFE wrappers / call-arg
/// callbacks. Mirrors the dispatcher walkers in
/// `crates/analyze/src/props.rs`.
fn collect_rewrite_insertions(
    stmt: &Statement<'_>,
    ctor_locals: &[String],
    out: &mut Vec<(usize, &'static str)>,
) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for declarator in &decl.declarations {
                if !matches!(&declarator.id, BindingPattern::BindingIdentifier(_)) {
                    continue;
                }
                let Some(init) = &declarator.init else {
                    continue;
                };
                if let Expression::CallExpression(call) = init
                    && let Expression::Identifier(callee_id) = &call.callee
                    && ctor_locals.iter().any(|n| n == callee_id.name.as_str())
                    && call.type_arguments.is_none()
                {
                    out.push((callee_id.span.end as usize, "<__SvnCustomEvents<$$Events>>"));
                }
                for s in stmts_in_function_expr(init) {
                    collect_rewrite_insertions(s, ctor_locals, out);
                }
            }
        }
        Statement::FunctionDeclaration(fd) => {
            if let Some(body) = &fd.body {
                for s in &body.statements {
                    collect_rewrite_insertions(s, ctor_locals, out);
                }
            }
        }
        Statement::IfStatement(s) => {
            // Round-14 #1: walk function-body stmts inside the if-test
            // expression too. An untyped dispatcher decl hidden in an
            // IIFE used as the test condition needs the typed-events
            // rewrite or its `dispatch(...)` calls go un-checked.
            for s2 in stmts_in_function_expr(&s.test) {
                collect_rewrite_insertions(s2, ctor_locals, out);
            }
            collect_rewrite_insertions(&s.consequent, ctor_locals, out);
            if let Some(alt) = &s.alternate {
                collect_rewrite_insertions(alt, ctor_locals, out);
            }
        }
        Statement::BlockStatement(b) => {
            for s in &b.body {
                collect_rewrite_insertions(s, ctor_locals, out);
            }
        }
        Statement::ExpressionStatement(es) => {
            for s in stmts_in_function_expr(&es.expression) {
                collect_rewrite_insertions(s, ctor_locals, out);
            }
        }
        Statement::ForStatement(s) => {
            if let Some(init) = &s.init {
                use oxc_ast::ast::ForStatementInit;
                match init {
                    ForStatementInit::VariableDeclaration(decl) => {
                        for d in &decl.declarations {
                            if !matches!(&d.id, BindingPattern::BindingIdentifier(_)) {
                                continue;
                            }
                            let Some(d_init) = &d.init else { continue };
                            if let Expression::CallExpression(call) = d_init
                                && let Expression::Identifier(callee_id) = &call.callee
                                && ctor_locals.iter().any(|n| n == callee_id.name.as_str())
                                && call.type_arguments.is_none()
                            {
                                out.push((
                                    callee_id.span.end as usize,
                                    "<__SvnCustomEvents<$$Events>>",
                                ));
                            }
                            for s2 in stmts_in_function_expr(d_init) {
                                collect_rewrite_insertions(s2, ctor_locals, out);
                            }
                        }
                    }
                    other => {
                        if let Some(expr) = other.as_expression() {
                            for s2 in stmts_in_function_expr(expr) {
                                collect_rewrite_insertions(s2, ctor_locals, out);
                            }
                        }
                    }
                }
            }
            collect_rewrite_insertions(&s.body, ctor_locals, out);
        }
        Statement::ForInStatement(s) => collect_rewrite_insertions(&s.body, ctor_locals, out),
        Statement::ForOfStatement(s) => collect_rewrite_insertions(&s.body, ctor_locals, out),
        Statement::WhileStatement(s) => collect_rewrite_insertions(&s.body, ctor_locals, out),
        Statement::DoWhileStatement(s) => collect_rewrite_insertions(&s.body, ctor_locals, out),
        Statement::SwitchStatement(s) => {
            for case in &s.cases {
                for stmt in &case.consequent {
                    collect_rewrite_insertions(stmt, ctor_locals, out);
                }
            }
        }
        Statement::TryStatement(s) => {
            for stmt in &s.block.body {
                collect_rewrite_insertions(stmt, ctor_locals, out);
            }
            if let Some(handler) = &s.handler {
                for stmt in &handler.body.body {
                    collect_rewrite_insertions(stmt, ctor_locals, out);
                }
            }
            if let Some(finalizer) = &s.finalizer {
                for stmt in &finalizer.body {
                    collect_rewrite_insertions(stmt, ctor_locals, out);
                }
            }
        }
        Statement::LabeledStatement(s) => collect_rewrite_insertions(&s.body, ctor_locals, out),
        _ => {}
    }
}

/// Yield the body statements of nested function/arrow expressions
/// (including IIFE-wrapped + callback-arg shapes). Mirrors
/// `props.rs::statements_inside_function_expr`.
fn stmts_in_function_expr<'a, 'b>(expr: &'a Expression<'b>) -> Vec<&'a Statement<'b>> {
    let mut out = Vec::new();
    collect_function_body_stmts(expr, &mut out);
    out
}

fn collect_function_body_stmts<'a, 'b>(expr: &'a Expression<'b>, out: &mut Vec<&'a Statement<'b>>) {
    match expr {
        Expression::ArrowFunctionExpression(arrow) => {
            for s in &arrow.body.statements {
                out.push(s);
            }
        }
        Expression::FunctionExpression(fe) => {
            if let Some(body) = &fe.body {
                for s in &body.statements {
                    out.push(s);
                }
            }
        }
        Expression::ParenthesizedExpression(p) => {
            collect_function_body_stmts(&p.expression, out);
        }
        Expression::CallExpression(call) => {
            collect_function_body_stmts(&call.callee, out);
            for arg in &call.arguments {
                if let Some(arg_expr) = arg.as_expression() {
                    collect_function_body_stmts(arg_expr, out);
                }
            }
        }
        _ => {}
    }
}

/// Same shape as `crates/analyze/src/props.rs::collect_ctor_locals`,
/// inlined here to avoid pulling the analyze crate into the
/// rewrite path. Only imports whose source is exactly `'svelte'`
/// count — local functions and non-Svelte imports named
/// `createEventDispatcher` don't trigger the rewrite.
fn collect_ctor_locals(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for stmt in &program.body {
        let Statement::ImportDeclaration(decl) = stmt else {
            continue;
        };
        if decl.source.value.as_str() != "svelte" {
            continue;
        }
        let Some(specifiers) = &decl.specifiers else {
            continue;
        };
        for spec in specifiers {
            let oxc_ast::ast::ImportDeclarationSpecifier::ImportSpecifier(s) = spec else {
                continue;
            };
            let imported = match &s.imported {
                oxc_ast::ast::ModuleExportName::IdentifierName(n) => n.name.as_str(),
                oxc_ast::ast::ModuleExportName::IdentifierReference(r) => r.name.as_str(),
                oxc_ast::ast::ModuleExportName::StringLiteral(l) => l.value.as_str(),
            };
            if imported == "createEventDispatcher" {
                out.push(s.local.name.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        rewrite(src, ScriptLang::Ts)
    }

    #[test]
    fn rewrites_untyped_dispatcher() {
        let src = "import { createEventDispatcher } from 'svelte';\n\
                   const dispatch = createEventDispatcher();";
        assert_eq!(
            ts(src),
            "import { createEventDispatcher } from 'svelte';\n\
             const dispatch = createEventDispatcher<__SvnCustomEvents<$$Events>>();"
        );
    }

    #[test]
    fn leaves_typed_dispatcher_alone() {
        let src = "import { createEventDispatcher } from 'svelte';\n\
                   const dispatch = createEventDispatcher<{ foo: string }>();";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn skips_local_function_with_same_name() {
        let src = "function createEventDispatcher() { return null; }\n\
                   const d = createEventDispatcher();";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn skips_non_svelte_import() {
        let src = "import { createEventDispatcher } from 'some-other-pkg';\n\
                   const d = createEventDispatcher();";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn handles_aliased_import() {
        let src = "import { createEventDispatcher as ced } from 'svelte';\n\
                   const d = ced();";
        assert_eq!(
            ts(src),
            "import { createEventDispatcher as ced } from 'svelte';\n\
             const d = ced<__SvnCustomEvents<$$Events>>();"
        );
    }
}
