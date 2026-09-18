//! Compiler errors the analysis phase raises outside its AST walks,
//! from what the finished scopes say about the component
//! (`2-analyze/index.js`).

use oxc_ast::ast::{ModuleExportName, Program, Statement};
use svn_core::Range;
use svn_parser::ast::{Fragment, Node};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::scope::BindingKind;

/// Runes mode has no `$$props` / `$$restProps`: the compiler rejects
/// the first reference to either (`$$props` first) before it walks the
/// scripts.
pub(crate) fn legacy_props(ctx: &mut LintContext<'_>) {
    if !ctx.runes {
        return;
    }
    let Some(tree) = &ctx.scope_tree else {
        return;
    };
    let first = |name: &str| {
        tree.unresolved_refs
            .iter()
            .filter(|r| r.name == name)
            .map(|r| r.range)
            .min_by_key(|r| r.start)
    };
    let found = first("$$props")
        .map(|range| {
            (
                Code::legacy_props_invalid,
                messages::legacy_props_invalid(),
                range,
            )
        })
        .or_else(|| {
            first("$$restProps").map(|range| {
                (
                    Code::legacy_rest_props_invalid,
                    messages::legacy_rest_props_invalid(),
                    range,
                )
            })
        });
    if let Some((code, message, range)) = found {
        ctx.emit_error(code, message, range);
    }
}

/// `export { name }` in `<script module>` must name something the
/// module script declares, or a snippet the compiler can hoist there —
/// one at the top level of the template using nothing from the
/// instance script. Checked once every walk is done.
pub(crate) fn module_exports(
    program: &Program<'_>,
    base: u32,
    fragment: &Fragment,
    ctx: &mut LintContext<'_>,
) {
    let Some(tree) = &ctx.scope_tree else {
        return;
    };
    let mut snippets: Vec<(&str, Range, bool)> = Vec::new();
    collect_snippets(fragment, true, &mut snippets);
    let mut error: Option<(Code, String, Range)> = None;
    'outer: for stmt in &program.body {
        let Statement::ExportNamedDeclaration(decl) = stmt else {
            continue;
        };
        if decl.export_kind.is_type() {
            continue;
        }
        for spec in &decl.specifiers {
            if spec.export_kind.is_type() {
                continue;
            }
            let ModuleExportName::IdentifierReference(local) = &spec.local else {
                continue;
            };
            let name = local.name.as_str();
            if tree.resolve(tree.module_root, name).is_some() {
                continue;
            }
            let range = Range::new(spec.span.start + base, spec.span.end + base);
            let snippet = snippets.iter().find(|(n, _, _)| *n == name);
            let hoisted = snippet.is_some_and(|(_, snippet_range, top_level)| {
                *top_level && !uses_instance_bindings(tree, *snippet_range)
            });
            if hoisted {
                continue;
            }
            error = Some(if snippet.is_some() {
                (
                    Code::snippet_invalid_export,
                    messages::snippet_invalid_export(),
                    range,
                )
            } else {
                (
                    Code::export_undefined,
                    messages::export_undefined(name),
                    range,
                )
            });
            break 'outer;
        }
    }
    if let Some((code, message, range)) = error {
        ctx.emit_error(code, message, range);
    }
}

/// Whether anything inside `range` refers to a binding of the instance
/// script (which keeps a snippet from being hoisted to the module).
fn uses_instance_bindings(tree: &crate::scope::ScopeTree, range: Range) -> bool {
    tree.all_bindings().any(|(_, b)| {
        b.scope == tree.instance_root
            && b.kind != BindingKind::StoreSub
            && b.references
                .iter()
                .any(|r| r.range.start >= range.start && r.range.end <= range.end)
    })
}

/// Every `{#snippet}` in the template: its name, range, and whether it
/// sits at the template's top level.
fn collect_snippets<'a>(fragment: &'a Fragment, top: bool, out: &mut Vec<(&'a str, Range, bool)>) {
    for node in &fragment.nodes {
        match node {
            Node::SnippetBlock(b) => {
                out.push((b.name.as_str(), b.range, top));
                collect_snippets(&b.body, false, out);
            }
            Node::Element(e) => collect_snippets(&e.children, false, out),
            Node::Component(c) => collect_snippets(&c.children, false, out),
            Node::SvelteElement(s) => collect_snippets(&s.children, false, out),
            Node::IfBlock(b) => {
                collect_snippets(&b.consequent, false, out);
                for arm in &b.elseif_arms {
                    collect_snippets(&arm.body, false, out);
                }
                if let Some(alt) = &b.alternate {
                    collect_snippets(alt, false, out);
                }
            }
            Node::EachBlock(b) => {
                collect_snippets(&b.body, false, out);
                if let Some(alt) = &b.alternate {
                    collect_snippets(alt, false, out);
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(p) = &b.pending {
                    collect_snippets(p, false, out);
                }
                if let Some(t) = &b.then_branch {
                    collect_snippets(&t.body, false, out);
                }
                if let Some(c) = &b.catch_branch {
                    collect_snippets(&c.body, false, out);
                }
            }
            Node::KeyBlock(b) => collect_snippets(&b.body, false, out),
            Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
        }
    }
}

/// Outside runes mode the compiler orders the instance script's `$:`
/// statements by what each assigns and depends on, and rejects a
/// dependency cycle (`order_reactive_statements`). A statement depends
/// on every outer binding it references other than as the target of a
/// plain `=` assignment; each assigned name depends on each dependency
/// the statement does not also assign. The first cycle a depth-first
/// search finds is reported on the first statement assigning its first
/// name.
pub(crate) fn reactive_cycle(ctx: &mut LintContext<'_>) {
    use crate::scope::{BindingId, ScopeId, ScopeTree};
    if ctx.runes {
        return;
    }
    let Some(tree) = &ctx.scope_tree else {
        return;
    };
    if tree.reactive_statements.len() < 2 {
        return;
    }
    let within = |tree: &ScopeTree, mut scope: ScopeId, outer: ScopeId| loop {
        if scope == outer {
            return true;
        }
        match tree.scope(scope).parent {
            Some(parent) => scope = parent,
            None => return false,
        }
    };
    // Per statement: the bindings it assigns and the ones it depends
    // on, each in first-seen order.
    let mut statements: Vec<(Range, Vec<BindingId>, Vec<BindingId>)> = Vec::new();
    for stmt in &tree.reactive_statements {
        let mut assignments: Vec<BindingId> = Vec::new();
        for (name, scope) in &stmt.assignments {
            if let Some(b) = tree.resolve(*scope, name)
                && !assignments.contains(&b)
            {
                assignments.push(b);
            }
        }
        let mut dependencies: Vec<(u32, BindingId)> = Vec::new();
        for (id, binding) in tree.all_bindings() {
            if within(tree, binding.scope, stmt.scope) {
                continue;
            }
            let inside = binding
                .references
                .iter()
                .filter(|r| r.range.start >= stmt.range.start && r.range.end <= stmt.range.end);
            let first = inside.clone().map(|r| r.range.start).min();
            let depends = inside
                .clone()
                .any(|r| !tree.reactive_assignment_targets.contains(&r.range.start));
            if let (Some(first), true) = (first, depends) {
                dependencies.push((first, id));
            }
        }
        dependencies.sort_by_key(|(first, _)| *first);
        statements.push((
            stmt.range,
            assignments,
            dependencies.into_iter().map(|(_, id)| id).collect(),
        ));
    }
    let name = |b: BindingId| tree.binding(b).name.clone();
    let mut edges: Vec<(smol_str::SmolStr, smol_str::SmolStr)> = Vec::new();
    for (_, assignments, dependencies) in &statements {
        for a in assignments {
            for d in dependencies {
                if !assignments.contains(d) {
                    edges.push((name(*a), name(*d)));
                }
            }
        }
    }
    let Some(cycle) = first_cycle(&edges) else {
        return;
    };
    let Some((range, _, _)) = statements
        .iter()
        .find(|(_, assignments, _)| assignments.iter().any(|a| name(*a) == cycle[0]))
    else {
        return;
    };
    let path: Vec<&str> = cycle.iter().map(|n| n.as_str()).collect();
    let range = *range;
    ctx.emit_error(
        Code::reactive_declaration_cycle,
        messages::reactive_declaration_cycle(&path.join(" → ")),
        range,
    );
}

/// The compiler's `check_graph_for_cycles`: a depth-first search from
/// each node in first-seen order; a cycle is the whole current search
/// path plus the node it closes on.
fn first_cycle(edges: &[(smol_str::SmolStr, smol_str::SmolStr)]) -> Option<Vec<smol_str::SmolStr>> {
    use smol_str::SmolStr;
    let mut nodes: Vec<SmolStr> = Vec::new();
    let mut graph: Vec<Vec<usize>> = Vec::new();
    let index = |n: &SmolStr, nodes: &mut Vec<SmolStr>, graph: &mut Vec<Vec<usize>>| {
        if let Some(i) = nodes.iter().position(|m| m == n) {
            i
        } else {
            nodes.push(n.clone());
            graph.push(Vec::new());
            nodes.len() - 1
        }
    };
    for (u, v) in edges {
        let u = index(u, &mut nodes, &mut graph);
        let v = index(v, &mut nodes, &mut graph);
        graph[u].push(v);
    }
    let mut visited = vec![false; nodes.len()];
    let mut stack: Vec<usize> = Vec::new();
    let mut cycles: Vec<Vec<usize>> = Vec::new();
    fn visit(
        v: usize,
        graph: &[Vec<usize>],
        visited: &mut [bool],
        stack: &mut Vec<usize>,
        cycles: &mut Vec<Vec<usize>>,
    ) {
        visited[v] = true;
        stack.push(v);
        for &w in &graph[v] {
            if !visited[w] {
                visit(w, graph, visited, stack, cycles);
            } else if stack.contains(&w) {
                let mut cycle = stack.clone();
                cycle.push(w);
                cycles.push(cycle);
            }
        }
        stack.pop();
    }
    for v in 0..nodes.len() {
        if !visited[v] {
            visit(v, &graph, &mut visited, &mut stack, &mut cycles);
        }
    }
    cycles
        .into_iter()
        .next()
        .map(|c| c.into_iter().map(|i| nodes[i].clone()).collect())
}
