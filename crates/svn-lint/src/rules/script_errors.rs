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
