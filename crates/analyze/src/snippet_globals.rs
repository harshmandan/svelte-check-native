//! The free names of a root `{#snippet}`: every identifier its
//! expressions reference that nothing inside the snippet declares,
//! plus the components it renders.
//!
//! Mirrors what upstream feeds `rootSnippets` in
//! `htmlxtojsx_v2/index.ts`: periscopic's `analyze` of the snippet as
//! a function (`result.globals`) merged with
//! `collectSnippetComponentGlobals`. Upstream then hoists the snippet
//! to module scope when every such name is allowed there — i.e. none
//! of them is declared by the instance script.

use std::collections::HashSet;

use oxc_ast::ast::IdentifierReference;
use oxc_ast_visit::{Visit, walk};
use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{Component, Element, Fragment, Node, SnippetBlock, SvelteElement};

use crate::template_refs::attribute_expression_sites;
use crate::template_scope::{BoundIdent, ScopeKind, TemplateScopeVisitor, walk_with_visitor};

/// Free names referenced by `snippet` (its parameters and every name
/// bound inside it excluded), in first-seen order.
pub fn snippet_globals(snippet: &SnippetBlock, source: &str) -> Vec<SmolStr> {
    // Walk the snippet as a one-node fragment so the scope walker
    // enters its parameter scope itself.
    let fragment = Fragment {
        nodes: vec![Node::SnippetBlock(Box::new(snippet.clone()))],
        ..Fragment::default()
    };
    let mut nested = HashSet::new();
    collect_snippet_names(&snippet.body, &mut nested);
    let mut collector = GlobalsCollector {
        source,
        scopes: vec![nested],
        seen: HashSet::new(),
        out: Vec::new(),
    };
    walk_with_visitor(&fragment, source, &mut collector);
    collector.out
}

/// Names of every `{#snippet}` declared anywhere under `fragment` —
/// a nested snippet is a declaration of its enclosing block, so a
/// reference to it is never free.
fn collect_snippet_names(fragment: &Fragment, out: &mut HashSet<SmolStr>) {
    for node in &fragment.nodes {
        match node {
            Node::SnippetBlock(b) => {
                out.insert(b.name.clone());
                collect_snippet_names(&b.body, out);
            }
            Node::Element(e) => collect_snippet_names(&e.children, out),
            Node::Component(c) => collect_snippet_names(&c.children, out),
            Node::SvelteElement(e) => collect_snippet_names(&e.children, out),
            Node::IfBlock(b) => {
                collect_snippet_names(&b.consequent, out);
                for arm in &b.elseif_arms {
                    collect_snippet_names(&arm.body, out);
                }
                if let Some(alt) = &b.alternate {
                    collect_snippet_names(alt, out);
                }
            }
            Node::EachBlock(b) => {
                collect_snippet_names(&b.body, out);
                if let Some(alt) = &b.alternate {
                    collect_snippet_names(alt, out);
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(p) = &b.pending {
                    collect_snippet_names(p, out);
                }
                if let Some(t) = &b.then_branch {
                    collect_snippet_names(&t.body, out);
                }
                if let Some(c) = &b.catch_branch {
                    collect_snippet_names(&c.body, out);
                }
            }
            Node::KeyBlock(b) => collect_snippet_names(&b.body, out),
            Node::Text(_) | Node::Comment(_) | Node::Interpolation(_) => {}
        }
    }
}

struct GlobalsCollector<'s> {
    source: &'s str,
    /// One frame per open scope; a frame holds the names it binds.
    scopes: Vec<HashSet<SmolStr>>,
    seen: HashSet<SmolStr>,
    out: Vec<SmolStr>,
}

impl GlobalsCollector<'_> {
    fn is_bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|frame| frame.contains(name))
    }

    fn reference(&mut self, name: &str) {
        if self.is_bound(name) || self.seen.contains(name) {
            return;
        }
        let name = SmolStr::from(name);
        self.seen.insert(name.clone());
        self.out.push(name);
    }

    fn expression(&mut self, range: Range, is_declaration: bool) {
        let Some(text) = self.source.get(range.start as usize..range.end as usize) else {
            return;
        };
        let wrapped = if is_declaration {
            format!("let {text}\n;")
        } else {
            format!("({text}\n);")
        };
        let alloc = oxc_allocator::Allocator::default();
        let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
        let mut refs = RefNames(Vec::new());
        refs.visit_program(&parsed.program);
        for name in refs.0 {
            self.reference(&name);
        }
    }

    fn attributes(&mut self, attrs: &[svn_parser::Attribute]) {
        for site in attribute_expression_sites(attrs) {
            match site {
                crate::template_refs::AttributeSite::Expression(range) => {
                    self.expression(range, false);
                }
                crate::template_refs::AttributeSite::Shorthand(name) => self.reference(&name),
            }
        }
    }

    /// `<Comp>` / `<ui.Comp>` / `<svelte:component this={Comp}>` — the
    /// root identifier is a value reference (upstream
    /// `collectSnippetComponentGlobals`).
    fn component_name(&mut self, name: &str) {
        let root = name.split('.').next().unwrap_or(name);
        if !root.is_empty() {
            self.reference(root);
        }
    }
}

impl TemplateScopeVisitor for GlobalsCollector<'_> {
    fn enter_fragment(&mut self) {
        self.scopes.push(HashSet::new());
    }

    fn leave_fragment(&mut self) {
        self.scopes.pop();
    }

    fn enter_scope(&mut self, _kind: ScopeKind, bindings: &[BoundIdent], _scope_range: Range) {
        self.scopes
            .push(bindings.iter().map(|b| b.name.clone()).collect());
    }

    fn leave_scope(&mut self, _kind: ScopeKind) {
        self.scopes.pop();
    }

    fn visit_element(&mut self, element: &Element) {
        self.attributes(&element.attributes);
    }

    fn visit_component(&mut self, component: &Component) {
        self.component_name(component.name.as_str());
        self.attributes(&component.attributes);
    }

    fn visit_svelte_element(&mut self, element: &SvelteElement) {
        self.attributes(&element.attributes);
    }

    fn visit_expr(&mut self, range: Range) {
        self.expression(range, false);
    }

    fn visit_at_const(&mut self, bound_names: &[SmolStr], expr_range: Range) {
        self.expression(expr_range, true);
        if let Some(frame) = self.scopes.last_mut() {
            frame.extend(bound_names.iter().cloned());
        }
    }
}

/// Every identifier reference in a parsed expression, in source order.
struct RefNames(Vec<SmolStr>);

impl<'a> Visit<'a> for RefNames {
    fn visit_identifier_reference(&mut self, it: &IdentifierReference<'a>) {
        self.0.push(SmolStr::from(it.name.as_str()));
        walk::walk_identifier_reference(self, it);
    }
}

#[cfg(test)]
mod tests {
    use super::snippet_globals;

    fn globals(src: &str) -> Vec<String> {
        let (doc, _) = svn_parser::parse_sections(src);
        let (fragment, _) = svn_parser::parse_all_template_runs(src, &doc.template.text_runs);
        let snippet = fragment
            .nodes
            .iter()
            .find_map(|n| match n {
                svn_parser::Node::SnippetBlock(b) => Some(b.as_ref()),
                _ => None,
            })
            .expect("a root snippet");
        snippet_globals(snippet, src)
            .into_iter()
            .map(|s| s.to_string())
            .collect()
    }

    #[test]
    fn params_and_inner_bindings_are_not_globals() {
        let g = globals(
            "{#snippet row(item: Item, i)}{#each item.kids as kid}{@const k = kid.id}<Card {kid} label={k} onclick={() => open(i)} />{/each}{@render inner()}{#snippet inner()}{count}{/snippet}{/snippet}",
        );
        assert_eq!(g, vec!["Card", "open", "count"]);
    }
}
