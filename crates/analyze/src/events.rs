//! Event-dispatcher analysis — the `createEventDispatcher` side of a
//! component's public surface.
//!
//! Mirrors upstream's split between
//! `svelte2tsx/src/svelte2tsx/nodes/ComponentEvents.ts` (the typed
//! `createEventDispatcher<T>()` source extraction) and the
//! untyped-event-name collection that feeds the `$$Events` synthesis.
//! Kept separate from `props.rs` (which mirrors `ExportedNames.ts`)
//! because Props and Events are independent concerns: a component can
//! have either, both, or neither, and emit consumes them through
//! distinct paths.
//!
//! The functions here detect:
//!
//! - the local name(s) a dispatcher was bound to
//!   (`const dispatch = createEventDispatcher()`), including aliased
//!   imports and the typed vs untyped distinction;
//! - every event name passed to a `dispatch('name')` call, walking all
//!   expression positions (not just bare call statements);
//! - the `<T>` type-argument source slices of typed dispatchers, which
//!   the caller chains into the `$$Events` type.

use oxc_ast::ast::{BindingPattern, Expression, Statement};
use oxc_span::GetSpan;

/// SVELTE-4-COMPAT — typed-events narrowing source.
///
/// Find every assigned `createEventDispatcher<T>()` call (top-level
/// and nested under function/block/if bodies) in `program` and
/// return the source slices of each `T` in declaration order. Caller
/// chains them into a source-order spread (`Omit<T1, keyof T2> & T2 …`)
/// so multi-dispatcher components mirror upstream
/// `ComponentEvents.toDefString()`'s
/// `...__sveltets_2_toEventTypings<T>()` shape (see round-9 #2).
///
/// Resolves aliased imports (`import { createEventDispatcher as d }`)
/// so `d<T>()` calls also match. Untyped `createEventDispatcher()`
/// calls are silently skipped here — caller picks them up via
/// `find_dispatcher_local_names` + `find_dispatched_event_names`.
///
/// Reviewer follow-up #3: pre-fix this returned only the FIRST
/// typed dispatcher's `<T>` and the caller's `or_else` chain
/// suppressed untyped dispatched-name synthesis whenever any typed
/// dispatcher existed — the multi-dispatcher and mixed-typed-untyped
/// cases lost their event signatures entirely.
pub fn find_dispatcher_event_type_sources(
    program: &oxc_ast::ast::Program<'_>,
    source: &str,
) -> Vec<String> {
    let ctor_locals = collect_ctor_locals(program);
    let mut out = Vec::new();
    // Round-9 follow-up #3: only count dispatchers ASSIGNED to a
    // BindingIdentifier (`const x = createEventDispatcher<T>()`) —
    // matches upstream's `processInstanceScriptContent.ts:271`
    // requirement (the dispatcher must be reachable as a callable
    // identifier for `dispatch('foo', …)` later). Bare expression
    // statements like `createEventDispatcher<{foo: string}>();` get
    // dropped because upstream never reaches them via
    // `setEventDispatcher`. Walk recursively through function/block/
    // if bodies so nested declarations land too.
    let mut handle_var_decl = |decl: &oxc_ast::ast::VariableDeclaration<'_>| {
        for d in &decl.declarations {
            if !matches!(&d.id, BindingPattern::BindingIdentifier(_)) {
                continue;
            }
            let Some(init) = &d.init else { continue };
            if let Some(slice) = dispatcher_type_arg_slice(init, source, &ctor_locals) {
                out.push(slice);
            }
        }
    };
    for stmt in &program.body {
        crate::ast_walk::walk_statement_descend(stmt, &mut |node| match node {
            crate::ast_walk::WalkNode::Statement(Statement::VariableDeclaration(decl)) => {
                handle_var_decl(decl);
            }
            crate::ast_walk::WalkNode::Statement(Statement::ExportNamedDeclaration(ed)) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) = &ed.declaration
                {
                    handle_var_decl(decl);
                }
            }
            crate::ast_walk::WalkNode::ForInitVarDecl(decl) => handle_var_decl(decl),
            _ => {}
        });
    }
    out
}

/// Find the typed-dispatcher locals — the subset of
/// `find_dispatcher_local_names` whose `createEventDispatcher`
/// call was given an explicit `<T>` type argument.
///
/// Used by emit to compute the UNTYPED-only dispatcher locals: the
/// difference (`all_locals \ typed_locals`) is then scanned with
/// `find_dispatched_event_names` to pull out untyped-dispatched
/// names without double-counting names already covered by typed
/// dispatcher type args.
pub fn find_typed_dispatcher_local_names(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    let ctor_locals = collect_ctor_locals(program);
    let mut out = Vec::new();
    collect_dispatcher_locals_via_walker(program, &ctor_locals, true, &mut out);
    out
}

/// Find local names bound to a `createEventDispatcher(...)` call at
/// top level: `const NAME = createEventDispatcher(...)` (any
/// type-arg form). Resolves `import { createEventDispatcher as
/// alias }` so `alias()` is also recognised. Returns ALL such
/// bindings — multiple dispatchers per file are allowed.
///
/// Used by [`find_dispatched_event_names`] to scope the
/// event-name scan to actual dispatcher calls.
pub fn find_dispatcher_local_names(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    let ctor_locals = collect_ctor_locals(program);
    let mut out = Vec::new();
    collect_dispatcher_locals_via_walker(program, &ctor_locals, false, &mut out);
    out
}

/// Round-11 follow-up #3: find local names bound to an UNTYPED
/// `createEventDispatcher()` call (no `<T>` type argument). Mirrors
/// upstream's per-call-typed-vs-untyped check
/// (`ComponentEvents.ts:256-264`): a call like `dispatch('foo', …)`
/// only contributes 'foo' to the events surface when at least one
/// dispatcher binding with that NAME is untyped.
///
/// Pre-fix native computed `untyped_locals` as
/// `find_dispatcher_local_names \ find_typed_dispatcher_local_names`
/// by name, which wrongly excluded a top-level untyped dispatcher
/// SHADOWED by a nested typed dispatcher with the same name. This
/// helper instead lists names that have AT LEAST ONE untyped
/// binding anywhere in the script (regardless of whether other
/// bindings with the same name are typed) — matching upstream's
/// `eventDispatchers.some(d => !d.typing && d.name === call.name)`
/// check.
pub fn find_untyped_dispatcher_local_names(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    let ctor_locals = collect_ctor_locals(program);
    let mut all: Vec<String> = Vec::new();
    collect_dispatcher_locals_via_walker(program, &ctor_locals, false, &mut all);
    let mut typed: Vec<String> = Vec::new();
    collect_dispatcher_locals_via_walker(program, &ctor_locals, true, &mut typed);
    // A name has at least one untyped binding iff its multiset
    // count in `all` exceeds its count in `typed` — same name can
    // appear once typed AND once untyped under shadowing, and the
    // untyped binding still makes the name "reachable" as untyped.
    let mut counts_all: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for n in &all {
        *counts_all.entry(n.clone()).or_default() += 1;
    }
    let mut counts_typed: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for n in &typed {
        *counts_typed.entry(n.clone()).or_default() += 1;
    }
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for n in all {
        let total = counts_all.get(&n).copied().unwrap_or(0);
        let typed_count = counts_typed.get(&n).copied().unwrap_or(0);
        if total > typed_count && seen.insert(n.clone()) {
            out.push(n);
        }
    }
    out
}

/// Walk every VariableDeclaration in `program` (including nested,
/// in for-init slots, inside function bodies, in catch handlers,
/// etc.) and append the names of `BindingIdentifier`s whose init is
/// a `<ctor-local>(...)` dispatcher call. When `typed_only` is true,
/// only declarators whose call has an explicit `<T>` type argument
/// contribute.
///
/// Driven by [`crate::ast_walk::walk_statement_descend`] — every
/// VariableDeclaration in the program reaches the closure once, no
/// matter where the AST hides it (block bodies, loop init slots,
/// switch case bodies, IIFEs, exported wrappers). Adding a new
/// container means adding a single descent arm to `walk_statement_
/// descend`, not 7 walker arms.
fn collect_dispatcher_locals_via_walker(
    program: &oxc_ast::ast::Program<'_>,
    ctor_locals: &std::collections::HashSet<String>,
    typed_only: bool,
    out: &mut Vec<String>,
) {
    let mut handle_var_decl = |decl: &oxc_ast::ast::VariableDeclaration<'_>| {
        for d in &decl.declarations {
            let Some(init) = &d.init else { continue };
            let Expression::CallExpression(call) = init else {
                continue;
            };
            let Expression::Identifier(id) = &call.callee else {
                continue;
            };
            if !ctor_locals.contains(id.name.as_str()) {
                continue;
            }
            if typed_only && call.type_arguments.is_none() {
                continue;
            }
            if let BindingPattern::BindingIdentifier(bid) = &d.id {
                out.push(bid.name.to_string());
            }
        }
    };
    for stmt in &program.body {
        crate::ast_walk::walk_statement_descend(stmt, &mut |node| match node {
            crate::ast_walk::WalkNode::Statement(Statement::VariableDeclaration(decl)) => {
                handle_var_decl(decl);
            }
            crate::ast_walk::WalkNode::Statement(Statement::ExportNamedDeclaration(ed)) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) = &ed.declaration
                {
                    handle_var_decl(decl);
                }
            }
            crate::ast_walk::WalkNode::ForInitVarDecl(decl) => handle_var_decl(decl),
            _ => {}
        });
    }
}

/// Collect the set of locals that resolve to svelte's
/// `createEventDispatcher`. Limited to imports whose source is
/// exactly `'svelte'` — covers the un-aliased
/// `import { createEventDispatcher } from 'svelte'`, the aliased
/// `import { createEventDispatcher as <local> } from 'svelte'`,
/// and the namespace `import * as <ns> from 'svelte'` form
/// (consumers call `ns.createEventDispatcher`, but our existing
/// callsites match `Identifier(callee)` and don't traverse member
/// expressions, so namespace imports are out of scope today).
///
/// Reviewer follow-up #4: pre-fix this also unconditionally
/// inserted the bare name on a "Svelte tooling injects it"
/// rationale that no fixture or upstream sample actually exercises.
/// Mirrors upstream `ComponentEvents.ts:386-389` exactly: only
/// imports from `'svelte'` count. Without this gate, a local
/// function (or non-Svelte import) named `createEventDispatcher`
/// would force dispatcher detection, event surface synthesis, and
/// the iso default-export shape on a value that has no actual
/// Svelte event semantics.
pub fn collect_ctor_locals(program: &oxc_ast::ast::Program<'_>) -> std::collections::HashSet<String> {
    use std::collections::HashSet;
    let mut ctor_locals: HashSet<String> = HashSet::new();
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
            // `imported` is the source-side name; `local` is the
            // value the user calls in this module. For the
            // un-aliased form they're identical.
            let imported = match &s.imported {
                oxc_ast::ast::ModuleExportName::IdentifierName(n) => n.name.as_str(),
                oxc_ast::ast::ModuleExportName::IdentifierReference(r) => r.name.as_str(),
                oxc_ast::ast::ModuleExportName::StringLiteral(l) => l.value.as_str(),
            };
            if imported == "createEventDispatcher" {
                ctor_locals.insert(s.local.name.to_string());
            }
        }
    }
    ctor_locals
}

/// Scan `program` for `<dispatcher>(<string-literal>, ...)` calls
/// where `<dispatcher>` is one of `dispatcher_locals`. Returns the
/// union of distinct event-name string literals in source order.
///
/// Used by the untyped-dispatcher synth path
/// (`<script strictEvents>` or runes mode without `interface
/// $$Events` and without an explicit `createEventDispatcher<T>()`
/// type arg). Each name flows into a synthesized `type $$Events =
/// { name1: any, name2: any, … }` so the consumer-side `$on('name',
/// cb)` resolves cb to `(e: any) => any` — narrowed from "any
/// string" to the actual dispatched-name set.
pub fn find_dispatched_event_names(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    use std::collections::{HashMap, HashSet};
    // Round-11 follow-up #2: single source-order walk that grows
    // `literal_vars` as we encounter `const NAME = 'literal'`
    // bindings. Pre-fix native ran a separate pre-pass that
    // populated literal_vars globally before any dispatched-name
    // scan — that overcounted FORWARD references.
    //
    // Round-13 follow-up #1: the same single-pass tracking now
    // applies to UNTYPED DISPATCHER LOCALS — pre-round-13 native
    // pre-collected them via `find_untyped_dispatcher_local_names`
    // so a forward call `dispatch('ready'); const dispatch =
    // createEventDispatcher();` wrongly registered 'ready'. Now
    // `dispatcher_locals_seen` grows as we encounter each untyped
    // dispatcher decl, and call-site checks consult the
    // then-current state.
    //
    // We register the literal binding AND the dispatcher binding
    // BEFORE walking the init expression — matching upstream's
    // `processInstanceScriptContent.ts:271` visit-then-recurse
    // ordering. For `const X = 'literal'` the init has no nested
    // calls; for `const X = (() => { dispatch(X) })()` the init's
    // IIFE body is walked AFTER X is registered (if X had been a
    // string literal — here it isn't, so X never registers).
    let ctor_locals = collect_ctor_locals(program);
    let mut literal_vars: HashMap<String, String> = HashMap::new();
    let mut dispatcher_locals_seen: HashSet<String> = HashSet::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();

    let scan_var_decl = |decl: &oxc_ast::ast::VariableDeclaration<'_>,
                         dispatcher_locals: &mut HashSet<String>,
                         literal_vars: &mut HashMap<String, String>,
                         seen: &mut HashSet<String>,
                         out: &mut Vec<String>| {
        for d in &decl.declarations {
            // Round-15 #1: drop the const-only restriction.
            // Upstream's `getVariableAtTopLevel` walks every
            // VariableDeclaration regardless of kind, so
            // `let EV = 'save'; dispatch(EV)` resolves the alias the
            // same way as `const EV = 'save'`. The `let` form is
            // technically reassignable, but upstream doesn't gate on
            // that and we don't either.
            if let BindingPattern::BindingIdentifier(bid) = &d.id
                && let Some(Expression::StringLiteral(s)) = &d.init
            {
                literal_vars.insert(bid.name.to_string(), s.value.to_string());
            }
            // Round-13 follow-up #1: track untyped dispatcher locals
            // incrementally. A forward call `dispatch('ready'); const
            // dispatch = createEventDispatcher();` doesn't see
            // `dispatch` in dispatcher_locals at the call site — so
            // 'ready' doesn't register. Pre-fix native pre-collected
            // every untyped dispatcher and accepted forward refs.
            if let BindingPattern::BindingIdentifier(bid) = &d.id
                && let Some(Expression::CallExpression(call)) = &d.init
                && let Expression::Identifier(callee_id) = &call.callee
                && ctor_locals.contains(callee_id.name.as_str())
                && call.type_arguments.is_none()
            {
                dispatcher_locals.insert(bid.name.to_string());
            }
            if let Some(init) = &d.init {
                scan_expression_for_dispatched_names(
                    init,
                    dispatcher_locals,
                    literal_vars,
                    seen,
                    out,
                );
            }
        }
    };

    for stmt in &program.body {
        crate::ast_walk::walk_statement_descend(stmt, &mut |node| match node {
            crate::ast_walk::WalkNode::Statement(Statement::VariableDeclaration(decl)) => {
                scan_var_decl(
                    decl,
                    &mut dispatcher_locals_seen,
                    &mut literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::ExportNamedDeclaration(ed)) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) = &ed.declaration
                {
                    scan_var_decl(
                        decl,
                        &mut dispatcher_locals_seen,
                        &mut literal_vars,
                        &mut seen,
                        &mut out,
                    );
                }
            }
            crate::ast_walk::WalkNode::ForInitVarDecl(decl) => {
                scan_var_decl(
                    decl,
                    &mut dispatcher_locals_seen,
                    &mut literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::ExpressionStatement(es)) => {
                scan_expression_for_dispatched_names(
                    &es.expression,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::ReturnStatement(rs)) => {
                if let Some(arg) = &rs.argument {
                    scan_expression_for_dispatched_names(
                        arg,
                        &dispatcher_locals_seen,
                        &literal_vars,
                        &mut seen,
                        &mut out,
                    );
                }
            }
            // Loop / switch headers — `while (dispatch('e'))`, etc.
            // The walker descends into IIFEs in these positions on its
            // own (via collect_function_body_stmts). The closure handles
            // the bare expression scan upstream's
            // `processInstanceScriptContent.ts:271` does at each header.
            crate::ast_walk::WalkNode::Statement(Statement::IfStatement(s)) => {
                scan_expression_for_dispatched_names(
                    &s.test,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::ForStatement(s)) => {
                if let Some(test) = &s.test {
                    scan_expression_for_dispatched_names(
                        test,
                        &dispatcher_locals_seen,
                        &literal_vars,
                        &mut seen,
                        &mut out,
                    );
                }
                if let Some(update) = &s.update {
                    scan_expression_for_dispatched_names(
                        update,
                        &dispatcher_locals_seen,
                        &literal_vars,
                        &mut seen,
                        &mut out,
                    );
                }
                if let Some(init) = &s.init
                    && let Some(e) = init.as_expression()
                {
                    scan_expression_for_dispatched_names(
                        e,
                        &dispatcher_locals_seen,
                        &literal_vars,
                        &mut seen,
                        &mut out,
                    );
                }
            }
            crate::ast_walk::WalkNode::Statement(Statement::ForInStatement(s)) => {
                scan_expression_for_dispatched_names(
                    &s.right,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::ForOfStatement(s)) => {
                scan_expression_for_dispatched_names(
                    &s.right,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::WhileStatement(s)) => {
                scan_expression_for_dispatched_names(
                    &s.test,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::DoWhileStatement(s)) => {
                scan_expression_for_dispatched_names(
                    &s.test,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
            }
            crate::ast_walk::WalkNode::Statement(Statement::SwitchStatement(s)) => {
                scan_expression_for_dispatched_names(
                    &s.discriminant,
                    &dispatcher_locals_seen,
                    &literal_vars,
                    &mut seen,
                    &mut out,
                );
                for case in &s.cases {
                    if let Some(test) = &case.test {
                        scan_expression_for_dispatched_names(
                            test,
                            &dispatcher_locals_seen,
                            &literal_vars,
                            &mut seen,
                            &mut out,
                        );
                    }
                }
            }
            _ => {}
        });
    }
    out
}

fn scan_statement_for_dispatched_names(
    stmt: &Statement<'_>,
    dispatcher_locals: &std::collections::HashSet<String>,
    literal_vars: &std::collections::HashMap<String, String>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    match stmt {
        Statement::VariableDeclaration(decl) => {
            for d in &decl.declarations {
                if let Some(init) = &d.init {
                    scan_expression_for_dispatched_names(
                        init,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                }
            }
        }
        Statement::ExpressionStatement(es) => {
            scan_expression_for_dispatched_names(
                &es.expression,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
        }
        Statement::FunctionDeclaration(fd) => {
            if let Some(body) = &fd.body {
                for s in &body.statements {
                    scan_statement_for_dispatched_names(
                        s,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                }
            }
        }
        Statement::ReturnStatement(rs) => {
            if let Some(arg) = &rs.argument {
                scan_expression_for_dispatched_names(
                    arg,
                    dispatcher_locals,
                    literal_vars,
                    seen,
                    out,
                );
            }
        }
        Statement::IfStatement(s) => {
            scan_expression_for_dispatched_names(
                &s.test,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
            scan_statement_for_dispatched_names(
                &s.consequent,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
            if let Some(alt) = &s.alternate {
                scan_statement_for_dispatched_names(
                    alt,
                    dispatcher_locals,
                    literal_vars,
                    seen,
                    out,
                );
            }
        }
        Statement::BlockStatement(b) => {
            for s in &b.body {
                scan_statement_for_dispatched_names(s, dispatcher_locals, literal_vars, seen, out);
            }
        }
        _ => {}
    }
}

fn scan_expression_for_dispatched_names(
    expr: &Expression<'_>,
    dispatcher_locals: &std::collections::HashSet<String>,
    literal_vars: &std::collections::HashMap<String, String>,
    seen: &mut std::collections::HashSet<String>,
    out: &mut Vec<String>,
) {
    match expr {
        Expression::CallExpression(call) => {
            // Match `<local>(<arg>, ...)` where <local> is a known
            // dispatcher binding. The <arg> is the event name —
            // either a direct string literal, or an identifier
            // whose binding resolved to a string literal at module
            // top level (#3c slice).
            if let Expression::Identifier(id) = &call.callee
                && dispatcher_locals.contains(id.name.as_str())
                && let Some(first) = call.arguments.first()
                && let Some(first_expr) = first.as_expression()
            {
                let resolved = match first_expr {
                    Expression::StringLiteral(s) => Some(s.value.to_string()),
                    Expression::Identifier(id) => literal_vars.get(id.name.as_str()).cloned(),
                    _ => None,
                };
                if let Some(name) = resolved
                    && seen.insert(name.clone())
                {
                    out.push(name);
                }
            }
            // Recurse into callee + args to catch nested calls
            // (e.g. `wrap(dispatch('foo', payload))`).
            scan_expression_for_dispatched_names(
                &call.callee,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
            for a in &call.arguments {
                if let Some(e) = a.as_expression() {
                    scan_expression_for_dispatched_names(
                        e,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                } else if let oxc_ast::ast::Argument::SpreadElement(s) = a {
                    scan_expression_for_dispatched_names(
                        &s.argument,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                }
            }
        }
        Expression::ArrowFunctionExpression(arrow) => {
            for s in &arrow.body.statements {
                scan_statement_for_dispatched_names(s, dispatcher_locals, literal_vars, seen, out);
            }
        }
        Expression::FunctionExpression(fe) => {
            if let Some(body) = &fe.body {
                for s in &body.statements {
                    scan_statement_for_dispatched_names(
                        s,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                }
            }
        }
        // Descend through expression-OPERATOR positions so a `dispatch(…)`
        // call anywhere in a compound expression is found — not just as a
        // bare statement or inside a function body. Upstream visits every
        // CallExpression in the script via a whole-AST `forEachChild`
        // walk, so it records the event name regardless of position; the
        // common top-level idiom `isValid && dispatch('submit')` (a
        // LogicalExpression statement) was previously dropped here.
        Expression::LogicalExpression(e) => {
            let mut go = |x| {
                scan_expression_for_dispatched_names(x, dispatcher_locals, literal_vars, seen, out)
            };
            go(&e.left);
            go(&e.right);
        }
        Expression::BinaryExpression(e) => {
            let mut go = |x| {
                scan_expression_for_dispatched_names(x, dispatcher_locals, literal_vars, seen, out)
            };
            go(&e.left);
            go(&e.right);
        }
        Expression::ConditionalExpression(e) => {
            let mut go = |x| {
                scan_expression_for_dispatched_names(x, dispatcher_locals, literal_vars, seen, out)
            };
            go(&e.test);
            go(&e.consequent);
            go(&e.alternate);
        }
        Expression::SequenceExpression(e) => {
            for x in &e.expressions {
                scan_expression_for_dispatched_names(x, dispatcher_locals, literal_vars, seen, out);
            }
        }
        Expression::ParenthesizedExpression(e) => scan_expression_for_dispatched_names(
            &e.expression,
            dispatcher_locals,
            literal_vars,
            seen,
            out,
        ),
        Expression::AwaitExpression(e) => scan_expression_for_dispatched_names(
            &e.argument,
            dispatcher_locals,
            literal_vars,
            seen,
            out,
        ),
        Expression::UnaryExpression(e) => scan_expression_for_dispatched_names(
            &e.argument,
            dispatcher_locals,
            literal_vars,
            seen,
            out,
        ),
        Expression::AssignmentExpression(e) => scan_expression_for_dispatched_names(
            &e.right,
            dispatcher_locals,
            literal_vars,
            seen,
            out,
        ),
        // `{ key: dispatch('x'), [dispatch('k')]: v, ...spreadCall() }` —
        // descend into property values, computed keys, and spread args.
        Expression::ObjectExpression(o) => {
            let mut go = |x| {
                scan_expression_for_dispatched_names(x, dispatcher_locals, literal_vars, seen, out)
            };
            for prop in &o.properties {
                match prop {
                    oxc_ast::ast::ObjectPropertyKind::ObjectProperty(p) => {
                        if p.computed
                            && let Some(key_expr) = p.key.as_expression()
                        {
                            go(key_expr);
                        }
                        go(&p.value);
                    }
                    oxc_ast::ast::ObjectPropertyKind::SpreadProperty(s) => {
                        go(&s.argument);
                    }
                }
            }
        }
        // `[dispatch('x'), ...spreadCall()]` — elements and spread args.
        Expression::ArrayExpression(a) => {
            let mut go = |x| {
                scan_expression_for_dispatched_names(x, dispatcher_locals, literal_vars, seen, out)
            };
            for elem in &a.elements {
                if let Some(e) = elem.as_expression() {
                    go(e);
                } else if let oxc_ast::ast::ArrayExpressionElement::SpreadElement(s) = elem {
                    go(&s.argument);
                }
            }
        }
        // `new Foo(dispatch('x'))` — callee and constructor args.
        Expression::NewExpression(call) => {
            scan_expression_for_dispatched_names(
                &call.callee,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
            for a in &call.arguments {
                if let Some(e) = a.as_expression() {
                    scan_expression_for_dispatched_names(
                        e,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                } else if let oxc_ast::ast::Argument::SpreadElement(s) = a {
                    scan_expression_for_dispatched_names(
                        &s.argument,
                        dispatcher_locals,
                        literal_vars,
                        seen,
                        out,
                    );
                }
            }
        }
        // `` `${dispatch('x')}` `` — template substitutions.
        Expression::TemplateLiteral(t) => {
            for e in &t.expressions {
                scan_expression_for_dispatched_names(e, dispatcher_locals, literal_vars, seen, out);
            }
        }
        // `` tag`${dispatch('x')}` `` — tag and template substitutions.
        Expression::TaggedTemplateExpression(t) => {
            scan_expression_for_dispatched_names(
                &t.tag,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
            for e in &t.quasi.expressions {
                scan_expression_for_dispatched_names(e, dispatcher_locals, literal_vars, seen, out);
            }
        }
        // `obj.prop` / `obj[dispatch('k')]` — object and computed key.
        Expression::StaticMemberExpression(me) => scan_expression_for_dispatched_names(
            &me.object,
            dispatcher_locals,
            literal_vars,
            seen,
            out,
        ),
        Expression::ComputedMemberExpression(me) => {
            scan_expression_for_dispatched_names(
                &me.object,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
            scan_expression_for_dispatched_names(
                &me.expression,
                dispatcher_locals,
                literal_vars,
                seen,
                out,
            );
        }
        _ => {}
    }
}

/// AST-based check: does `program` contain at least one call to
/// `createEventDispatcher(...)` (typed or untyped, top-level or
/// nested in an initializer / function body)? Resolves aliased
/// imports (`import { createEventDispatcher as d }`) so `d()` is
/// also counted (#3b slice). Used by the default-export shape
/// decision to choose between the
/// `__sveltets_2_fn_component`-equivalent `Component<P, X, B>`
/// shape (no events) and the iso-component interface (events
/// present).
///
/// Substring detection on raw source text false-positives on
/// comments (`// uses createEventDispatcher`), string literals, and
/// unused imports — none of which actually emit events. The AST
/// walk only fires on real call expressions.
pub fn has_event_dispatcher_call(program: &oxc_ast::ast::Program<'_>) -> bool {
    let ctor_locals = collect_ctor_locals(program);
    let mut found = false;
    for stmt in &program.body {
        crate::ast_walk::walk_statement_descend(stmt, &mut |node| {
            if found {
                return;
            }
            let exprs: Vec<&Expression<'_>> = match node {
                crate::ast_walk::WalkNode::Statement(s) => statement_local_exprs(s),
                crate::ast_walk::WalkNode::ForInitVarDecl(decl) => decl
                    .declarations
                    .iter()
                    .filter_map(|d| d.init.as_ref())
                    .collect(),
            };
            if exprs
                .iter()
                .any(|e| expression_has_dispatcher_call_local(e, &ctor_locals))
            {
                found = true;
            }
        });
        if found {
            return true;
        }
    }
    false
}

/// Surface the expressions that hang directly off `stmt` (init for a
/// VariableDeclaration / ExportNamedDeclaration's wrapped VarDecl,
/// the bare expression of an ExpressionStatement). The walker handles
/// descent into nested function bodies / control-flow children
/// separately, so this stays a non-recursive scan.
fn statement_local_exprs<'a, 'b>(stmt: &'a Statement<'b>) -> Vec<&'a Expression<'b>> {
    match stmt {
        Statement::VariableDeclaration(decl) => decl
            .declarations
            .iter()
            .filter_map(|d| d.init.as_ref())
            .collect(),
        Statement::ExpressionStatement(es) => vec![&es.expression],
        Statement::ExportNamedDeclaration(ed) => match &ed.declaration {
            Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) => decl
                .declarations
                .iter()
                .filter_map(|d| d.init.as_ref())
                .collect(),
            _ => Vec::new(),
        },
        _ => Vec::new(),
    }
}

/// Local-only expression scan: does `expr` contain a dispatcher call
/// at THIS expression level (or in a sub-expression that isn't a
/// function/arrow body)? The walker handles descent into nested
/// function bodies separately, so this stays a non-recursive scan
/// across structural expression operators.
fn expression_has_dispatcher_call_local(
    expr: &Expression<'_>,
    ctor_locals: &std::collections::HashSet<String>,
) -> bool {
    match expr {
        Expression::CallExpression(call) => {
            if let Expression::Identifier(id) = &call.callee
                && ctor_locals.contains(id.name.as_str())
            {
                return true;
            }
            if expression_has_dispatcher_call_local(&call.callee, ctor_locals) {
                return true;
            }
            call.arguments.iter().any(|a| {
                a.as_expression()
                    .is_some_and(|e| expression_has_dispatcher_call_local(e, ctor_locals))
            })
        }
        // Stop at function/arrow bodies — `walk_statement_descend`
        // surfaces those statements as separate visits.
        Expression::ArrowFunctionExpression(_) | Expression::FunctionExpression(_) => false,
        Expression::ParenthesizedExpression(p) => {
            expression_has_dispatcher_call_local(&p.expression, ctor_locals)
        }
        Expression::TSAsExpression(t) => {
            expression_has_dispatcher_call_local(&t.expression, ctor_locals)
        }
        Expression::TSSatisfiesExpression(t) => {
            expression_has_dispatcher_call_local(&t.expression, ctor_locals)
        }
        Expression::TSNonNullExpression(t) => {
            expression_has_dispatcher_call_local(&t.expression, ctor_locals)
        }
        _ => false,
    }
}

/// If `expr` is a `<dispatcher-local><T>(...)` call with an
/// explicit type argument, return `T`'s source text. The
/// `ctor_locals` set is the alias-aware list of names that resolve
/// to `createEventDispatcher` (see [`collect_ctor_locals`]).
fn dispatcher_type_arg_slice(
    expr: &Expression<'_>,
    source: &str,
    ctor_locals: &std::collections::HashSet<String>,
) -> Option<String> {
    let Expression::CallExpression(call) = expr else {
        return None;
    };
    match &call.callee {
        Expression::Identifier(id) if ctor_locals.contains(id.name.as_str()) => {}
        _ => return None,
    }
    let tp = call.type_arguments.as_ref()?;
    let arg = tp.params.first()?;
    let span = arg.span();
    source
        .get(span.start as usize..span.end as usize)
        .map(str::to_string)
}

/// Round-8 follow-up #5: collect every property-signature name from
/// every inline-type-literal typed dispatcher in this program. Used
/// by emit to detect names shared across multiple typed dispatchers
/// — upstream's `addToEvents` collapses such duplicates to
/// `CustomEvent<any>` via the dispatchedEvents-set override; we
/// route the duplicates through the untyped-names layer (which the
/// round-7 #5 layer order already overrides last with
/// `CustomEvent<any>`).
///
/// Returns names in walk order, with duplicates retained — caller
/// can dedupe to find the names that appeared >= 2 times.
pub fn collect_inline_typed_dispatcher_member_names(
    program: &oxc_ast::ast::Program<'_>,
) -> Vec<String> {
    let ctor_locals = collect_ctor_locals(program);
    // Round-14 #6: mirror upstream's `getIdentifierValue` /
    // `getVariableAtTopLevel` (`ComponentEvents.ts:319`) — computed
    // property keys like `[EVENT]` resolve against top-level
    // `const EVENT = 'literal'` declarations. We collect those
    // bindings once and pass them down so member-name extraction
    // sees the resolved literal.
    let literal_vars = collect_top_level_string_const_literals(program);
    let mut out = Vec::new();
    let mut handle_var_decl = |decl: &oxc_ast::ast::VariableDeclaration<'_>| {
        for d in &decl.declarations {
            if !matches!(&d.id, BindingPattern::BindingIdentifier(_)) {
                continue;
            }
            let Some(init) = &d.init else { continue };
            expression_collect_inline_typed_members(init, &ctor_locals, &literal_vars, &mut out);
        }
    };
    for stmt in &program.body {
        crate::ast_walk::walk_statement_descend(stmt, &mut |node| match node {
            crate::ast_walk::WalkNode::Statement(Statement::VariableDeclaration(decl)) => {
                handle_var_decl(decl);
            }
            crate::ast_walk::WalkNode::Statement(Statement::ExportNamedDeclaration(ed)) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) = &ed.declaration
                {
                    handle_var_decl(decl);
                }
            }
            crate::ast_walk::WalkNode::ForInitVarDecl(decl) => handle_var_decl(decl),
            _ => {}
        });
    }
    out
}

/// Round-14 #6: walk the program's TOP-LEVEL `const NAME = 'literal'`
/// bindings and return them as a name → value map. Mirrors upstream's
/// `getVariableAtTopLevel` (`ComponentEvents.ts:339`) which only
/// considers bindings at module scope when resolving computed
/// property names. Locals declared inside functions / blocks are
/// intentionally NOT walked — upstream doesn't see them either.
fn collect_top_level_string_const_literals(
    program: &oxc_ast::ast::Program<'_>,
) -> std::collections::HashMap<String, String> {
    let mut out = std::collections::HashMap::new();
    for stmt in &program.body {
        // Round-15 #1: accept both bare `let/const/var x = '…'` and
        // `export let/const/var x = '…'`. Upstream's
        // `getVariableAtTopLevel` (`ComponentEvents.ts:339`) walks
        // every top-level VariableDeclaration regardless of kind or
        // `export` wrapping; native used to require const-only and
        // skipped the export form entirely.
        let decl = match stmt {
            Statement::VariableDeclaration(decl) => decl.as_ref(),
            Statement::ExportNamedDeclaration(ed) => match &ed.declaration {
                Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) => decl.as_ref(),
                _ => continue,
            },
            _ => continue,
        };
        for d in &decl.declarations {
            let BindingPattern::BindingIdentifier(bid) = &d.id else {
                continue;
            };
            let Some(Expression::StringLiteral(s)) = &d.init else {
                continue;
            };
            out.insert(bid.name.to_string(), s.value.to_string());
        }
    }
    out
}

fn expression_collect_inline_typed_members(
    expr: &Expression<'_>,
    ctor_locals: &std::collections::HashSet<String>,
    literal_vars: &std::collections::HashMap<String, String>,
    out: &mut Vec<String>,
) {
    let Expression::CallExpression(call) = expr else {
        return;
    };
    let Expression::Identifier(id) = &call.callee else {
        return;
    };
    if !ctor_locals.contains(id.name.as_str()) {
        return;
    }
    let Some(tp) = call.type_arguments.as_ref() else {
        return;
    };
    let Some(arg) = tp.params.first() else {
        return;
    };
    let oxc_ast::ast::TSType::TSTypeLiteral(lit) = arg else {
        return;
    };
    for member in &lit.members {
        let oxc_ast::ast::TSSignature::TSPropertySignature(prop) = member else {
            continue;
        };
        // Round-14 #6 / Round-15 #5: computed `[EVENT]` keys resolve
        // ONLY against a top-level `const EVENT = 'literal'`. Upstream's
        // `getName` (`ComponentEvents.ts:319`) accepts computed names
        // exclusively for the `ComputedPropertyName(Identifier)` case —
        // computed string-literal `['foo']: T` and any other computed
        // form throw. Native can't propagate user-script syntax errors,
        // so the divergence is to NOT count those keys (silent skip
        // instead of throw). Pre-round-15 #5 native accepted computed
        // string-literal as if it were `'foo': T`; that gave us a
        // phantom name in the duplicate-collapse pass that upstream
        // doesn't see.
        let key_name = if prop.computed {
            match &prop.key {
                oxc_ast::ast::PropertyKey::Identifier(id) => {
                    literal_vars.get(id.name.as_str()).cloned()
                }
                _ => None,
            }
        } else {
            match &prop.key {
                oxc_ast::ast::PropertyKey::StaticIdentifier(id) => Some(id.name.to_string()),
                oxc_ast::ast::PropertyKey::StringLiteral(s) => Some(s.value.to_string()),
                _ => None,
            }
        };
        if let Some(name) = key_name {
            out.push(name);
        }
    }
}

/// Round-8 follow-up #4: does any typed `createEventDispatcher<T>()`
/// in this program have an INLINE type literal (`{foo: …}`) with at
/// least one property signature?
///
/// Mirrors upstream's `events.size > 0` from typed-dispatcher
/// processing (`ComponentEvents.ts:231`), which only counts events
/// when the typed arg's `members` is enumerable — i.e. a
/// `ts.TypeLiteral` node, NOT a `ts.TypeReference` to an alias. The
/// fn-shape gate consults this so a runes component with
/// `createEventDispatcher<MyEventMap>()` (typed but ref-only)
/// stays on the fn-component path; only inline-literal type args
/// make events.hasEvents() upstream and disqualify fn-shape.
pub fn has_inline_typed_dispatcher_members(program: &oxc_ast::ast::Program<'_>) -> bool {
    let ctor_locals = collect_ctor_locals(program);
    let mut found = false;
    let check_var_decl = |decl: &oxc_ast::ast::VariableDeclaration<'_>| -> bool {
        decl.declarations.iter().any(|d| {
            if !matches!(&d.id, BindingPattern::BindingIdentifier(_)) {
                return false;
            }
            let Some(init) = &d.init else { return false };
            expression_has_inline_typed_dispatcher(init, &ctor_locals)
        })
    };
    for stmt in &program.body {
        crate::ast_walk::walk_statement_descend(stmt, &mut |node| {
            if found {
                return;
            }
            match node {
                crate::ast_walk::WalkNode::Statement(Statement::VariableDeclaration(decl)) => {
                    if check_var_decl(decl) {
                        found = true;
                    }
                }
                crate::ast_walk::WalkNode::Statement(Statement::ExportNamedDeclaration(ed)) => {
                    if let Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) =
                        &ed.declaration
                        && check_var_decl(decl)
                    {
                        found = true;
                    }
                }
                crate::ast_walk::WalkNode::ForInitVarDecl(decl) => {
                    if check_var_decl(decl) {
                        found = true;
                    }
                }
                _ => {}
            }
        });
        if found {
            return true;
        }
    }
    false
}

fn expression_has_inline_typed_dispatcher(
    expr: &Expression<'_>,
    ctor_locals: &std::collections::HashSet<String>,
) -> bool {
    let Expression::CallExpression(call) = expr else {
        return false;
    };
    let Expression::Identifier(id) = &call.callee else {
        return false;
    };
    if !ctor_locals.contains(id.name.as_str()) {
        return false;
    }
    let Some(tp) = call.type_arguments.as_ref() else {
        return false;
    };
    let Some(arg) = tp.params.first() else {
        return false;
    };
    matches!(arg, oxc_ast::ast::TSType::TSTypeLiteral(lit) if !lit.members.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_allocator::Allocator;
    use svn_parser::{ScriptLang, parse_script_body};

    fn dispatched_names(src: &str) -> Vec<String> {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        find_dispatched_event_names(&parsed.program)
    }

    #[test]
    fn dispatch_found_in_operator_positions() {
        // `dispatch(…)` in a compound expression (not just a bare call or
        // inside a function body) must still register the event name —
        // upstream visits every CallExpression regardless of position.
        let setup = "import { createEventDispatcher } from 'svelte';\nconst dispatch = createEventDispatcher();\n";
        // top-level `cond && dispatch('e')` (LogicalExpression statement)
        assert_eq!(
            dispatched_names(&format!("{setup}ok && dispatch('submit');")),
            vec!["submit"]
        );
        // ternary
        assert_eq!(
            dispatched_names(&format!("{setup}ok ? dispatch('yes') : dispatch('no');")),
            vec!["yes", "no"]
        );
        // inside a function body, in an operator position
        assert_eq!(
            dispatched_names(&format!(
                "{setup}function h() {{ ok && dispatch('inner'); }}"
            )),
            vec!["inner"]
        );
    }
}
