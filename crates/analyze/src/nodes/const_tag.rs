//! `{@const}` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/ConstTag.ts`.
//!
//! Two concerns live here: [`visit_at_const`] tracks the names a const
//! tag introduces into template scope, and [`check_const_placement`]
//! validates that each `{@const}` sits where Svelte permits it (the
//! `const_tag_invalid_placement` diagnostic).

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::ast::{Attribute, InterpolationKind, Node, SvelteElementKind};

use crate::walker::AnalyzeVisitor;

/// Upstream's `const_tag_invalid_placement` message, verbatim
/// (`svelte/compiler` errors.js), including the trailing docs URL.
pub const CONST_TAG_INVALID_PLACEMENT_MSG: &str = "`{@const}` must be the immediate child of `{#snippet}`, `{#if}`, `{:else if}`, `{:else}`, `{#each}`, `{:then}`, `{:catch}`, `<svelte:fragment>`, `<svelte:boundary>` or `<Component>`\nhttps://svelte.dev/e/const_tag_invalid_placement";

/// A `{@const}` placed where upstream forbids it. Carries the source
/// range of the offending interpolation; the caller maps it to a
/// diagnostic position and attaches [`CONST_TAG_INVALID_PLACEMENT_MSG`].
#[derive(Debug, Clone, Copy)]
pub struct ConstPlacementError {
    pub range: Range,
}

/// Recursively validate `{@const}` placement in a fragment. `allowed`
/// is whether a const tag sitting *directly* in this fragment is legal
/// — i.e. whether the node that owns this fragment is one of upstream's
/// permitted grand-parents. Misplaced tags are pushed to `out`.
pub fn check_const_placement(
    nodes: &[Node],
    allowed: bool,
    out: &mut Vec<ConstPlacementError>,
) {
    for node in nodes {
        match node {
            Node::Interpolation(i) if i.kind == InterpolationKind::AtConst => {
                if !allowed {
                    out.push(ConstPlacementError { range: i.range });
                }
            }
            // Plain elements / `<svelte:element>` host a const only when
            // they carry a `slot` attribute (named-slot fill).
            Node::Element(el) => {
                check_const_placement(&el.children.nodes, has_slot_attr(&el.attributes), out);
            }
            // Components always host const tags.
            Node::Component(c) => {
                check_const_placement(&c.children.nodes, true, out);
            }
            Node::SvelteElement(se) => {
                // `<svelte:fragment>`, `<svelte:boundary>` and
                // `<svelte:component>` are permitted hosts; a
                // `<svelte:element>` is only when it carries `slot`.
                // Everything else (`<svelte:self>`, window/head/…) is
                // not — matching upstream's allow-list.
                let child_allowed = match se.kind {
                    SvelteElementKind::Fragment
                    | SvelteElementKind::Boundary
                    | SvelteElementKind::Component => true,
                    SvelteElementKind::Element => has_slot_attr(&se.attributes),
                    _ => false,
                };
                check_const_placement(&se.children.nodes, child_allowed, out);
            }
            Node::IfBlock(b) => {
                check_const_placement(&b.consequent.nodes, true, out);
                for arm in &b.elseif_arms {
                    check_const_placement(&arm.body.nodes, true, out);
                }
                if let Some(alt) = &b.alternate {
                    check_const_placement(&alt.nodes, true, out);
                }
            }
            Node::EachBlock(b) => {
                check_const_placement(&b.body.nodes, true, out);
                if let Some(alt) = &b.alternate {
                    check_const_placement(&alt.nodes, true, out);
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(pending) = &b.pending {
                    check_const_placement(&pending.nodes, true, out);
                }
                if let Some(then) = &b.then_branch {
                    check_const_placement(&then.body.nodes, true, out);
                }
                if let Some(catch) = &b.catch_branch {
                    check_const_placement(&catch.body.nodes, true, out);
                }
            }
            Node::KeyBlock(b) => {
                check_const_placement(&b.body.nodes, true, out);
            }
            Node::SnippetBlock(b) => {
                check_const_placement(&b.body.nodes, true, out);
            }
            Node::Text(_) | Node::Comment(_) | Node::Interpolation(_) => {}
        }
    }
}

/// Whether an attribute list carries a `slot` attribute — the marker
/// that lets a plain element / `<svelte:element>` host a `{@const}`
/// (named-slot fill). Mirrors upstream's `a.type === 'Attribute' &&
/// a.name === 'slot'`: plain, shorthand and `slot={…}` expression
/// forms all count; spreads and directives do not.
fn has_slot_attr(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|a| match a {
        Attribute::Plain(p) => p.name == "slot",
        Attribute::Expression(e) => e.name == "slot",
        Attribute::Shorthand(s) => s.name == "slot",
        Attribute::Spread(_) | Attribute::Directive(_) | Attribute::Comment(_) => false,
    })
}

pub(crate) fn visit_at_const(
    v: &mut AnalyzeVisitor<'_>,
    bound_names: &[SmolStr],
    _expr_range: Range,
) {
    // Push every bound name onto the shadow so subsequent
    // slot-attr / let-directive sites in the same fragment treat
    // them as scope-local. Destructure `{@const}` forms
    // (`{@const { a, b } = X}`) emit multiple names; bare
    // `{@const NAME = X}` emits one. The walker's fragment-level
    // bracket truncates them at exit.
    for name in bound_names {
        // `{@const NAME = expr}` introduces a template-scope
        // binding without a value source we can rewrite (the
        // initialiser walks in the parent scope, but the bound
        // name itself is opaque to the slot resolver). Push as
        // `None` — bound but unresolvable. Slot-attr collection
        // drops references rather than splicing module-scope.
        v.shadow.entries.push((name.clone(), None));
    }
}
