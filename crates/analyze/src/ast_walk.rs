//! Shared AST-walking helpers consumed by multiple analyze + emit passes.
//!
//! Per `notes/PARITY_TESTING_PLAN.md` P3, hand-rolled `match Statement::*`
//! recursion duplicates a generated visitor we don't have. The first piece
//! to pull out of the duplication is `collect_function_body_stmts`, which
//! had byte-identical copies in `analyze::props` and
//! `emit::dispatcher_typing_rewrite` — every R14/R15 review round had to
//! land its descent fix in two places. Keeping the recursion in one
//! module means a new oxc enum variant only adds one match arm, not seven.

use oxc_ast::ast::{Declaration, Expression, ForStatementInit, Statement, VariableDeclaration};

/// Walk an expression looking for nested function/arrow bodies — including
/// those passed as call arguments (`setTimeout(() => { … })`) and reachable
/// through every other expression form (object-literal values, array
/// elements, ternary branches, sequence expressions, TS-cast wrappers,
/// etc.). All recovered statements get flattened into `out` so each
/// dispatcher walker can iterate uniformly. Mirrors upstream's
/// `ts.forEachChild` descent pattern.
///
/// Stops at `ClassExpression` / `JSXElement` / `JSXFragment` — they have
/// their own scoping rules that the dispatcher walkers don't currently
/// model.
pub fn collect_function_body_stmts<'a, 'b>(
    expr: &'a Expression<'b>,
    out: &mut Vec<&'a Statement<'b>>,
) {
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
        Expression::ClassExpression(_) => {}
        Expression::JSXElement(_) | Expression::JSXFragment(_) => {}
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
        Expression::NewExpression(call) => {
            collect_function_body_stmts(&call.callee, out);
            for arg in &call.arguments {
                if let Some(arg_expr) = arg.as_expression() {
                    collect_function_body_stmts(arg_expr, out);
                }
            }
        }
        Expression::ConditionalExpression(c) => {
            collect_function_body_stmts(&c.test, out);
            collect_function_body_stmts(&c.consequent, out);
            collect_function_body_stmts(&c.alternate, out);
        }
        Expression::LogicalExpression(b) => {
            collect_function_body_stmts(&b.left, out);
            collect_function_body_stmts(&b.right, out);
        }
        Expression::BinaryExpression(b) => {
            collect_function_body_stmts(&b.left, out);
            collect_function_body_stmts(&b.right, out);
        }
        Expression::UnaryExpression(u) => {
            collect_function_body_stmts(&u.argument, out);
        }
        Expression::AwaitExpression(a) => {
            collect_function_body_stmts(&a.argument, out);
        }
        Expression::YieldExpression(y) => {
            if let Some(arg) = &y.argument {
                collect_function_body_stmts(arg, out);
            }
        }
        Expression::SequenceExpression(s) => {
            for e in &s.expressions {
                collect_function_body_stmts(e, out);
            }
        }
        Expression::AssignmentExpression(a) => {
            collect_function_body_stmts(&a.right, out);
        }
        Expression::ObjectExpression(o) => {
            for prop in &o.properties {
                match prop {
                    oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                        if p.computed
                            && let Some(key_expr) = p.key.as_expression()
                        {
                            collect_function_body_stmts(key_expr, out);
                        }
                        collect_function_body_stmts(&p.value, out);
                    }
                    oxc_ast::ast::ObjectPropertyKind::SpreadProperty(s) => {
                        collect_function_body_stmts(&s.argument, out);
                    }
                }
            }
        }
        Expression::ArrayExpression(a) => {
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    collect_function_body_stmts(e, out);
                } else if let oxc_ast::ast::ArrayExpressionElement::SpreadElement(s) = elem {
                    collect_function_body_stmts(&s.argument, out);
                }
            }
        }
        Expression::TemplateLiteral(t) => {
            for e in &t.expressions {
                collect_function_body_stmts(e, out);
            }
        }
        Expression::TaggedTemplateExpression(t) => {
            collect_function_body_stmts(&t.tag, out);
            for e in &t.quasi.expressions {
                collect_function_body_stmts(e, out);
            }
        }
        Expression::StaticMemberExpression(me) => {
            collect_function_body_stmts(&me.object, out);
        }
        Expression::ComputedMemberExpression(me) => {
            collect_function_body_stmts(&me.object, out);
            collect_function_body_stmts(&me.expression, out);
        }
        Expression::PrivateFieldExpression(me) => {
            collect_function_body_stmts(&me.object, out);
        }
        Expression::ImportExpression(i) => {
            collect_function_body_stmts(&i.source, out);
        }
        Expression::TSAsExpression(t) => {
            collect_function_body_stmts(&t.expression, out);
        }
        Expression::TSSatisfiesExpression(t) => {
            collect_function_body_stmts(&t.expression, out);
        }
        Expression::TSTypeAssertion(t) => {
            collect_function_body_stmts(&t.expression, out);
        }
        Expression::TSNonNullExpression(t) => {
            collect_function_body_stmts(&t.expression, out);
        }
        Expression::TSInstantiationExpression(t) => {
            collect_function_body_stmts(&t.expression, out);
        }
        // Round-Parity #4a: enumerate every remaining Expression
        // variant explicitly so the compiler enforces a decision when
        // oxc adds new variants. Pre-fix the wildcard `_ => {}` arm
        // would silently swallow new node kinds and the walker would
        // miss any function bodies they hold (the same regression
        // class P3 was meant to end).
        //
        // Optional chains: pierce into the underlying call/member.
        Expression::ChainExpression(c) => {
            collect_chain_element_stmts(&c.expression, out);
        }
        // PrivateIn (`#field in obj`): only the right side can hold
        // a function body (e.g. an object-expression containing
        // arrow values).
        Expression::PrivateInExpression(p) => {
            collect_function_body_stmts(&p.right, out);
        }
        // V8-intrinsic calls (`%FunctionCall(...)`): treat like a
        // call — args can hold function bodies.
        Expression::V8IntrinsicExpression(call) => {
            for arg in &call.arguments {
                if let Some(arg_expr) = arg.as_expression() {
                    collect_function_body_stmts(arg_expr, out);
                }
            }
        }
        // Update expressions (`++x` / `x--`): operand is a
        // SimpleAssignmentTarget — identifier-shaped, no function
        // bodies to descend into.
        Expression::UpdateExpression(_) => {}
        // Pure-leaf expressions: no nested children to discover.
        Expression::BooleanLiteral(_)
        | Expression::NullLiteral(_)
        | Expression::NumericLiteral(_)
        | Expression::BigIntLiteral(_)
        | Expression::RegExpLiteral(_)
        | Expression::StringLiteral(_)
        | Expression::Identifier(_)
        | Expression::MetaProperty(_)
        | Expression::Super(_)
        | Expression::ThisExpression(_) => {}
    }
}

/// Round-Parity #4a: ChainExpression's `expression` is a
/// `ChainElement` enum that inherits MemberExpression variants and
/// adds CallExpression / TSNonNullExpression. Map each arm into the
/// corresponding `Expression` descent.
fn collect_chain_element_stmts<'a, 'b>(
    elem: &'a oxc_ast::ast::ChainElement<'b>,
    out: &mut Vec<&'a Statement<'b>>,
) {
    use oxc_ast::ast::ChainElement;
    match elem {
        ChainElement::CallExpression(call) => {
            collect_function_body_stmts(&call.callee, out);
            for arg in &call.arguments {
                if let Some(arg_expr) = arg.as_expression() {
                    collect_function_body_stmts(arg_expr, out);
                }
            }
        }
        ChainElement::TSNonNullExpression(t) => {
            collect_function_body_stmts(&t.expression, out);
        }
        ChainElement::ComputedMemberExpression(me) => {
            collect_function_body_stmts(&me.object, out);
            collect_function_body_stmts(&me.expression, out);
        }
        ChainElement::StaticMemberExpression(me) => {
            collect_function_body_stmts(&me.object, out);
        }
        ChainElement::PrivateFieldExpression(me) => {
            collect_function_body_stmts(&me.object, out);
        }
    }
}

/// AST node yielded by [`walk_statement_descend`] — Statements plus
/// the one non-Statement binding context dispatcher walkers need
/// (`ForStatementInit::VariableDeclaration`, which oxc models
/// separately from regular Statements).
///
/// Adding a new variant when a future walker needs to react to
/// another binding context (e.g. catch-clause params, class-method
/// bodies, JSX tag attributes) is a single match arm here plus the
/// matching surfacing in [`walk_statement_descend`].
#[derive(Copy, Clone)]
pub enum WalkNode<'a, 'b> {
    /// Any oxc `Statement<'b>` reached through control-flow descent.
    Statement(&'a Statement<'b>),
    /// A `VariableDeclaration` that lives inside a `for (let x = …;
    /// …; …)` for-init slot. `ForStatementInit` isn't a `Statement`
    /// in oxc's AST, so callers that want to register binders at
    /// this position need a separate node type.
    ForInitVarDecl(&'a VariableDeclaration<'b>),
}

/// Visit `stmt` and every Statement reachable through control-flow
/// descent: block bodies, function bodies, if/else branches, loop
/// bodies, switch cases, try/catch/finally blocks, labeled-statement
/// bodies, AND IIFE bodies embedded in expressions of expression
/// statements / for-init/test/update / return arguments / if-tests
/// etc. ExportNamedDeclaration's wrapped Variable/Function declarations
/// surface as if they were direct `Statement::VariableDeclaration` /
/// `Statement::FunctionDeclaration` (the closure sees the synthesised
/// view through the same `&Statement` reference).
///
/// Visitation order: the parent statement is visited first, then its
/// children in source order. Callers that need source-order
/// observation (e.g. dispatcher-locals registration tracking
/// declaration sites) get it for free.
///
/// `f` is invoked on every node — the closure decides which variants
/// are interesting. ForStatement init's VariableDeclaration arrives
/// as `WalkNode::ForInitVarDecl` (separate from `Statement::Variable
/// Declaration`) so callers can react to binders in that position
/// without relying on `Statement::ForStatement` arms re-implementing
/// the same logic.
///
/// IMPORTANT: this is the descent surface every dispatcher /
/// slot-attr-rewrite walker SHOULD use instead of hand-rolling
/// `match Statement::*`. Per `notes/PARITY_TESTING_PLAN.md` P3, the
/// hand-rolled walkers were the source of repeated review rounds —
/// every new oxc enum variant was a silent miss in 7 places. This
/// helper keeps the enumeration in ONE place.
pub fn walk_statement_descend<'a, 'b, F>(stmt: &'a Statement<'b>, f: &mut F)
where
    F: FnMut(WalkNode<'a, 'b>),
{
    f(WalkNode::Statement(stmt));
    walk_statement_children(stmt, f);
}

fn walk_statement_children<'a, 'b, F>(stmt: &'a Statement<'b>, f: &mut F)
where
    F: FnMut(WalkNode<'a, 'b>),
{
    let mut iife_stmts: Vec<&'a Statement<'b>> = Vec::new();
    match stmt {
        Statement::BlockStatement(b) => {
            for s in &b.body {
                walk_statement_descend(s, f);
            }
        }
        Statement::FunctionDeclaration(fd) => {
            if let Some(body) = &fd.body {
                for s in &body.statements {
                    walk_statement_descend(s, f);
                }
            }
        }
        Statement::ExportNamedDeclaration(ed) => match &ed.declaration {
            Some(Declaration::FunctionDeclaration(fd)) => {
                if let Some(body) = &fd.body {
                    for s in &body.statements {
                        walk_statement_descend(s, f);
                    }
                }
            }
            Some(Declaration::VariableDeclaration(decl)) => {
                for d in &decl.declarations {
                    if let Some(init) = &d.init {
                        collect_function_body_stmts(init, &mut iife_stmts);
                    }
                }
            }
            _ => {}
        },
        Statement::IfStatement(s) => {
            collect_function_body_stmts(&s.test, &mut iife_stmts);
            walk_statement_descend(&s.consequent, f);
            if let Some(alt) = &s.alternate {
                walk_statement_descend(alt, f);
            }
        }
        Statement::ExpressionStatement(es) => {
            collect_function_body_stmts(&es.expression, &mut iife_stmts);
        }
        Statement::ReturnStatement(rs) => {
            if let Some(arg) = &rs.argument {
                collect_function_body_stmts(arg, &mut iife_stmts);
            }
        }
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let Some(init) = &d.init {
                    collect_function_body_stmts(init, &mut iife_stmts);
                }
            }
        }
        Statement::ForStatement(s) => {
            if let Some(init) = &s.init {
                match init {
                    ForStatementInit::VariableDeclaration(decl) => {
                        // Surface as ForInitVarDecl so binding-site
                        // walkers (statement_collect_dispatcher_locals,
                        // scan_var_decl_in_source_order) can register
                        // dispatchers declared in the for-init slot
                        // — `for (let d = createEventDispatcher();
                        // …; …) { d('save', …) }` should still flow
                        // through the same registration logic as a
                        // top-level `const d = …`.
                        f(WalkNode::ForInitVarDecl(decl.as_ref()));
                        for d in &decl.declarations {
                            if let Some(d_init) = &d.init {
                                collect_function_body_stmts(d_init, &mut iife_stmts);
                            }
                        }
                    }
                    other => {
                        if let Some(e) = other.as_expression() {
                            collect_function_body_stmts(e, &mut iife_stmts);
                        }
                    }
                }
            }
            if let Some(test) = &s.test {
                collect_function_body_stmts(test, &mut iife_stmts);
            }
            if let Some(update) = &s.update {
                collect_function_body_stmts(update, &mut iife_stmts);
            }
            walk_statement_descend(&s.body, f);
        }
        Statement::ForInStatement(s) => {
            collect_function_body_stmts(&s.right, &mut iife_stmts);
            walk_statement_descend(&s.body, f);
        }
        Statement::ForOfStatement(s) => {
            collect_function_body_stmts(&s.right, &mut iife_stmts);
            walk_statement_descend(&s.body, f);
        }
        Statement::WhileStatement(s) => {
            collect_function_body_stmts(&s.test, &mut iife_stmts);
            walk_statement_descend(&s.body, f);
        }
        Statement::DoWhileStatement(s) => {
            walk_statement_descend(&s.body, f);
            collect_function_body_stmts(&s.test, &mut iife_stmts);
        }
        Statement::SwitchStatement(s) => {
            collect_function_body_stmts(&s.discriminant, &mut iife_stmts);
            for case in &s.cases {
                if let Some(test) = &case.test {
                    collect_function_body_stmts(test, &mut iife_stmts);
                }
                for stmt in &case.consequent {
                    walk_statement_descend(stmt, f);
                }
            }
        }
        Statement::TryStatement(s) => {
            for stmt in &s.block.body {
                walk_statement_descend(stmt, f);
            }
            if let Some(handler) = &s.handler {
                for stmt in &handler.body.body {
                    walk_statement_descend(stmt, f);
                }
            }
            if let Some(finalizer) = &s.finalizer {
                for stmt in &finalizer.body {
                    walk_statement_descend(stmt, f);
                }
            }
        }
        Statement::LabeledStatement(s) => {
            walk_statement_descend(&s.body, f);
        }
        Statement::ThrowStatement(s) => {
            collect_function_body_stmts(&s.argument, &mut iife_stmts);
        }
        Statement::WithStatement(s) => {
            collect_function_body_stmts(&s.object, &mut iife_stmts);
            walk_statement_descend(&s.body, f);
        }
        // Explicit no-op arms below: every Statement variant gets a
        // line so a future oxc bump that adds a new variant fails to
        // compile until handled (rustc warns
        // `unreachable_patterns`/`non_exhaustive_omitted_patterns`
        // when a variant exists without a match arm). This is the
        // exhaustiveness-by-construction gate
        // `notes/PARITY_TESTING_PLAN.md` P3 calls for — every
        // dispatcher / slot-rewrite walker rides through this single
        // descent surface, so a missed variant fails ONE place
        // instead of seven.
        //
        // Bare statements: no nested Statement and no IIFE-bearing
        // expressions to recover.
        Statement::BreakStatement(_)
        | Statement::ContinueStatement(_)
        | Statement::DebuggerStatement(_)
        | Statement::EmptyStatement(_) => {}
        // Module declarations: ImportDeclaration is a leaf;
        // ExportNamedDeclaration is handled above (Statement::Export
        // NamedDeclaration arm); ExportAllDeclaration / ExportDefault
        // Declaration / TSExportAssignment / TSNamespaceExport
        // Declaration are leaves for our purposes (nothing dispatcher-
        // adjacent can hide inside them in a Svelte component
        // script).
        Statement::ImportDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::TSExportAssignment(_)
        | Statement::TSNamespaceExportDeclaration(_) => {}
        // ClassDeclaration: its body has its own scope; method bodies
        // never declare top-level dispatchers in a Svelte component
        // script. Static field initializers are skipped for the same
        // reason `collect_function_body_stmts` stops at
        // ClassExpression.
        Statement::ClassDeclaration(_) => {}
        // TS-only declarations: pure type-system constructs, no
        // runtime expressions to scan.
        Statement::TSTypeAliasDeclaration(_)
        | Statement::TSInterfaceDeclaration(_)
        | Statement::TSEnumDeclaration(_)
        | Statement::TSModuleDeclaration(_)
        | Statement::TSGlobalDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_) => {}
    }
    for s in iife_stmts {
        walk_statement_descend(s, f);
    }
}
