//! Which components sit in a slot position of a parent component.
//!
//! A text `slot="x"` on a component names the slot of its parent it
//! fills — but only when that parent is itself a component. svelte2tsx
//! decides this from the element it is building around the attribute
//! (`htmlxtojsx_v2/nodes/Attribute.ts`, `element.parent instanceof
//! InlineComponent`): every element and component opens a new element
//! whose parent is the one enclosing it, `{#snippet}` starts over with
//! none, and `{#if}` / `{#each}` / `{#await}` / `{#key}` leave it as it
//! is (`htmlxtojsx_v2/index.ts`). Anywhere else — inside a snippet, an
//! element, `<svelte:fragment>` or `<svelte:boundary>`, or at the top
//! level — `slot` is an ordinary prop of the component.

use std::collections::HashSet;

use svn_parser::{Fragment, Node, SvelteElementKind};

/// Start offsets of the components (`<Comp>`, `<svelte:component>`,
/// `<svelte:self>`) whose nearest enclosing element, component or
/// snippet is a component.
pub(crate) fn components_in_slot_position(fragment: &Fragment) -> HashSet<u32> {
    let mut out = HashSet::new();
    walk(fragment, false, &mut out);
    out
}

fn walk(fragment: &Fragment, parent_is_component: bool, out: &mut HashSet<u32>) {
    for node in &fragment.nodes {
        match node {
            Node::Component(c) => {
                if parent_is_component {
                    out.insert(c.range.start);
                }
                walk(&c.children, true, out);
            }
            Node::SvelteElement(s) => {
                let is_component = matches!(
                    s.kind,
                    SvelteElementKind::Component | SvelteElementKind::SelfRef
                );
                if is_component && parent_is_component {
                    out.insert(s.range.start);
                }
                walk(&s.children, is_component, out);
            }
            Node::Element(e) => walk(&e.children, false, out),
            Node::SnippetBlock(b) => walk(&b.body, false, out),
            Node::IfBlock(b) => {
                walk(&b.consequent, parent_is_component, out);
                for arm in &b.elseif_arms {
                    walk(&arm.body, parent_is_component, out);
                }
                if let Some(alternate) = &b.alternate {
                    walk(alternate, parent_is_component, out);
                }
            }
            Node::EachBlock(b) => {
                walk(&b.body, parent_is_component, out);
                if let Some(alternate) = &b.alternate {
                    walk(alternate, parent_is_component, out);
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(pending) = &b.pending {
                    walk(pending, parent_is_component, out);
                }
                if let Some(then) = &b.then_branch {
                    walk(&then.body, parent_is_component, out);
                }
                if let Some(catch) = &b.catch_branch {
                    walk(&catch.body, parent_is_component, out);
                }
            }
            Node::KeyBlock(b) => walk(&b.body, parent_is_component, out),
            Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn starts_of(source: &str, needle: &str) -> Vec<u32> {
        source
            .match_indices(needle)
            .map(|(i, _)| i as u32)
            .collect()
    }

    fn positioned(source: &str) -> HashSet<u32> {
        let doc = svn_parser::parse_sections(source).0;
        let fragment = svn_parser::parse_all_template_runs(source, &doc.template.text_runs).0;
        components_in_slot_position(&fragment)
    }

    #[test]
    fn direct_children_and_blocks_are_slot_positions() {
        let src = "<P><B slot=\"a\" />{#if x}<B slot=\"b\" />{/if}{#each xs as x}<B />{/each}</P>";
        let got = positioned(src);
        let expected: HashSet<u32> = starts_of(src, "<B").into_iter().collect();
        assert_eq!(got, expected);
    }

    #[test]
    fn snippets_and_elements_break_the_chain() {
        let src = "<P>{#snippet s()}<B slot=\"a\" />{/snippet}<div><B /></div><svelte:fragment slot=\"f\"><B /></svelte:fragment></P><B />";
        assert!(positioned(src).is_empty());
    }

    #[test]
    fn dynamic_components_count_as_parents_and_children() {
        let src = "<P><svelte:component this={C}><B /></svelte:component></P>";
        let got = positioned(src);
        assert!(got.contains(&3));
        assert!(got.contains(&(src.find("<B").unwrap() as u32)));
    }
}
