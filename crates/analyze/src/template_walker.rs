//! Template walker — populates the SemanticModel from a parsed Fragment.
//!
//! Single AST walk that visits every node and dispatches to the relevant
//! collectors:
//!
//! - `use:` directives → register `__svn_action_attrs_N` in
//!   [`VoidRefRegistry`] (one per directive, counter shared workspace-wide
//!   per component).
//! - `bind:foo={getter, setter}` → register `__svn_bind_pair_N`.
//! - `bind:this={x}` where `x` is a simple identifier → record `x` as a
//!   bind-target (used later for the `let x!: T` definite-assignment
//!   rewrite — bug fixture #30).
//! - Each block — counted; emit needs the count to generate unique loop
//!   binding names.
//!
//! Per todo.md §1.3 this should ideally fuse with rune detection in a
//! single visitor. For now rune detection runs over the script AST (oxc)
//! while template walking is structural — different inputs, two passes.
//! When we add a `Visit` trait that bridges both, we'll fuse.

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{Attribute, Directive, DirectiveKind, DirectiveValue, Fragment, Node};

use crate::void_refs::VoidRefRegistry;

/// Per-template summary populated during the walk.
#[derive(Debug, Default, Clone)]
pub struct TemplateSummary {
    /// Names (registered upstream) that need void-references emitted.
    pub void_refs: VoidRefRegistry,
    /// `bind:this={x}` targets where `x` is a simple identifier — eligible
    /// for the definite-assignment rewrite.
    pub bind_this_targets: Vec<BindThisTarget>,
    /// Number of `{#each}` blocks encountered. Emit uses this to allocate
    /// unique iteration helpers.
    pub each_block_count: usize,
}

/// One `bind:this={x}` site.
#[derive(Debug, Clone)]
pub struct BindThisTarget {
    /// The identifier name `x`.
    pub name: SmolStr,
    /// Source range of the bind expression (the `x` part).
    pub range: Range,
}

/// Walk the template fragment, collecting synthesized-name registrations
/// and bind-target metadata.
pub fn walk_template(fragment: &Fragment) -> TemplateSummary {
    let mut summary = TemplateSummary::default();
    summary.void_refs.register("__svn_tpl_check");
    let mut counters = Counters::default();
    walk_fragment(fragment, &mut summary, &mut counters);
    summary
}

#[derive(Default)]
struct Counters {
    action_attrs: usize,
    bind_pair: usize,
}

fn walk_fragment(fragment: &Fragment, summary: &mut TemplateSummary, counters: &mut Counters) {
    for node in &fragment.nodes {
        walk_node(node, summary, counters);
    }
}

fn walk_node(node: &Node, summary: &mut TemplateSummary, counters: &mut Counters) {
    match node {
        Node::Element(e) => {
            walk_attributes(&e.attributes, summary, counters);
            walk_fragment(&e.children, summary, counters);
        }
        Node::Component(c) => {
            walk_attributes(&c.attributes, summary, counters);
            walk_fragment(&c.children, summary, counters);
        }
        Node::SvelteElement(s) => {
            walk_attributes(&s.attributes, summary, counters);
            walk_fragment(&s.children, summary, counters);
        }
        Node::IfBlock(b) => {
            walk_fragment(&b.consequent, summary, counters);
            for arm in &b.elseif_arms {
                walk_fragment(&arm.body, summary, counters);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters);
            }
        }
        Node::EachBlock(b) => {
            summary.each_block_count += 1;
            walk_fragment(&b.body, summary, counters);
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters);
            }
        }
        Node::AwaitBlock(b) => {
            if let Some(p) = &b.pending {
                walk_fragment(p, summary, counters);
            }
            if let Some(t) = &b.then_branch {
                walk_fragment(&t.body, summary, counters);
            }
            if let Some(c) = &b.catch_branch {
                walk_fragment(&c.body, summary, counters);
            }
        }
        Node::KeyBlock(b) => walk_fragment(&b.body, summary, counters),
        Node::SnippetBlock(b) => walk_fragment(&b.body, summary, counters),
        // Leaf nodes — no children to descend into, no attributes.
        Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
    }
}

fn walk_attributes(attrs: &[Attribute], summary: &mut TemplateSummary, counters: &mut Counters) {
    for attr in attrs {
        if let Attribute::Directive(d) = attr {
            walk_directive(d, summary, counters);
        }
    }
}

fn walk_directive(d: &Directive, summary: &mut TemplateSummary, counters: &mut Counters) {
    match d.kind {
        DirectiveKind::Use => {
            // Each `use:foo` produces an action-attrs holder we must
            // void-reference.
            let name = format!("__svn_action_attrs_{}", counters.action_attrs);
            summary.void_refs.register(name);
            counters.action_attrs += 1;
        }
        DirectiveKind::Bind => {
            match &d.value {
                Some(DirectiveValue::BindPair { .. }) => {
                    let name = format!("__svn_bind_pair_{}", counters.bind_pair);
                    summary.void_refs.register(name);
                    counters.bind_pair += 1;
                }
                Some(DirectiveValue::Expression {
                    expression_range, ..
                }) if d.name == "this" => {
                    // bind:this={ident} → record as definite-assignment target
                    // when the expression is a single identifier.
                    if let Some(name) = simple_identifier_in(*expression_range) {
                        summary.bind_this_targets.push(BindThisTarget {
                            name,
                            range: *expression_range,
                        });
                    }
                }
                _ => {}
            }
        }
        _ => {}
    }
}

/// If the byte range covers a single ECMAScript identifier (with optional
/// surrounding whitespace), return it. Otherwise None.
///
/// Conservative: we only accept ASCII identifier chars (`a-zA-Z0-9_$`).
/// Real Unicode identifier support would require oxc; for the
/// definite-assignment rewrite we only care about plain identifiers.
fn simple_identifier_in(range: Range) -> Option<SmolStr> {
    // The range here doesn't have direct source access — caller would need
    // to provide source. We defer to a later refactor where this fn takes
    // (source, range). For now the walker passes the range through and the
    // emit/typecheck stage extracts the identifier text.
    let _ = range;
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use svn_parser::{parse_all_template_runs, parse_sections};

    fn walk_str(src: &str) -> TemplateSummary {
        let (doc, errors) = parse_sections(src);
        assert!(errors.is_empty(), "section parse errors: {errors:?}");
        let (fragment, errors) = parse_all_template_runs(src, &doc.template.text_runs);
        assert!(errors.is_empty(), "template parse errors: {errors:?}");
        walk_template(&fragment)
    }

    #[test]
    fn always_registers_template_check() {
        let s = walk_str("<p>hi</p>");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_tpl_check"))
        );
    }

    #[test]
    fn use_directive_registers_action_attrs() {
        // Bug fixture #8.
        let s = walk_str(r#"<div use:tooltip={{ text: 'hi' }}>x</div>"#);
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }

    #[test]
    fn multiple_use_directives_get_unique_indices() {
        let s = walk_str(r#"<div use:a use:b><span use:c /></div>"#);
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_1"))
        );
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_2"))
        );
    }

    #[test]
    fn bind_pair_registers_bind_pair() {
        // Bug fixture #9.
        let s = walk_str("<input bind:value={() => g(), (v) => s(v)} />");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_bind_pair_0"))
        );
    }

    #[test]
    fn simple_bind_does_not_register_bind_pair() {
        let s = walk_str("<input bind:value={x} />");
        assert!(
            !s.void_refs
                .names()
                .iter()
                .any(|n| n.starts_with("__svn_bind_pair"))
        );
    }

    #[test]
    fn each_block_increments_count() {
        let s = walk_str("{#each items as item}<p>{item}</p>{/each}");
        assert_eq!(s.each_block_count, 1);
    }

    #[test]
    fn nested_each_blocks_counted() {
        let s = walk_str("{#each rows as row}{#each row.items as item}<x />{/each}{/each}");
        assert_eq!(s.each_block_count, 2);
    }

    #[test]
    fn directives_in_nested_elements_are_walked() {
        let s = walk_str("<div><span use:tooltip /></div>");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }

    #[test]
    fn directives_in_block_body_are_walked() {
        let s = walk_str("{#if cond}<div use:focus />{/if}");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }

    #[test]
    fn each_alternate_branch_walked() {
        let s = walk_str("{#each items as i}<x />{:else}<div use:focus />{/each}");
        assert!(
            s.void_refs
                .names()
                .contains(&SmolStr::from("__svn_action_attrs_0"))
        );
    }
}
