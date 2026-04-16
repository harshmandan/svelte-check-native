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
//!   bind-target. Emit later rewrites the matching `let x: T;` declaration
//!   in the script to `let x!: T;` so TypeScript's definite-assignment
//!   analysis doesn't flag closure reads (Svelte assigns asynchronously).
//! - Each block — counted; emit needs the count to generate unique loop
//!   binding names.
//!
//! This should ideally fuse with rune detection in a single visitor.
//! For now rune detection runs over the script AST (oxc) while template
//! walking is structural — different inputs, two passes. When we add a
//! `Visit` trait that bridges both, we'll fuse.

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
    /// Names introduced by `{@const NAME = expr}` template tags. These
    /// are template-scope locals that don't exist in the script. Emit
    /// declares each as `let NAME: any;` inside `__svn_tpl_check` so
    /// downstream `{#if NAME.x}` / `{#each NAME as ...}` references
    /// don't fire TS2304.
    pub at_const_names: Vec<SmolStr>,
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
///
/// `source` is the original component source — needed to extract identifier
/// text from byte ranges (e.g. for `bind:this={x}`).
pub fn walk_template(fragment: &Fragment, source: &str) -> TemplateSummary {
    let mut summary = TemplateSummary::default();
    summary.void_refs.register("__svn_tpl_check");
    let mut counters = Counters::default();
    let mut ctx = WalkCtx { source };
    walk_fragment(fragment, &mut summary, &mut counters, &ctx);
    let _ = &mut ctx;
    collect_at_const_names(source, &mut summary.at_const_names);
    summary
}

/// Source-level scan for `{@const NAME = ...}` declarations.
///
/// The structural parser models `@const` as a generic interpolation
/// (we deliberately punt on per-tag semantics there), which loses the
/// LHS / RHS split. Re-scanning the raw source for the literal pattern
/// is the simplest path to the bound name without changing the AST
/// shape.
///
/// Multiple `{@const}` declarations and the same name across separate
/// templates are deduped so emit doesn't generate `let X: any;` twice
/// (TS2451 redeclaration).
fn collect_at_const_names(source: &str, out: &mut Vec<SmolStr>) {
    let bytes = source.as_bytes();
    let needle = b"{@const";
    let mut i = 0;
    let mut seen = std::collections::HashSet::<SmolStr>::new();
    while i + needle.len() < bytes.len() {
        if &bytes[i..i + needle.len()] != needle {
            i += 1;
            continue;
        }
        let mut p = i + needle.len();
        // Require whitespace after `@const`.
        if p >= bytes.len() || !bytes[p].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        while p < bytes.len() && bytes[p].is_ascii_whitespace() {
            p += 1;
        }
        let name_start = p;
        while p < bytes.len()
            && (bytes[p].is_ascii_alphanumeric() || bytes[p] == b'_' || bytes[p] == b'$')
        {
            p += 1;
        }
        if p == name_start {
            i += 1;
            continue;
        }
        let name = SmolStr::from(&source[name_start..p]);
        if seen.insert(name.clone()) {
            out.push(name);
        }
        i = p;
    }
}

struct WalkCtx<'src> {
    source: &'src str,
}

#[derive(Default)]
struct Counters {
    action_attrs: usize,
    bind_pair: usize,
}

fn walk_fragment(
    fragment: &Fragment,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    for node in &fragment.nodes {
        walk_node(node, summary, counters, ctx);
    }
}

fn walk_node(
    node: &Node,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    match node {
        Node::Element(e) => {
            walk_attributes(&e.attributes, summary, counters, ctx);
            walk_fragment(&e.children, summary, counters, ctx);
        }
        Node::Component(c) => {
            walk_attributes(&c.attributes, summary, counters, ctx);
            walk_fragment(&c.children, summary, counters, ctx);
        }
        Node::SvelteElement(s) => {
            walk_attributes(&s.attributes, summary, counters, ctx);
            walk_fragment(&s.children, summary, counters, ctx);
        }
        Node::IfBlock(b) => {
            walk_fragment(&b.consequent, summary, counters, ctx);
            for arm in &b.elseif_arms {
                walk_fragment(&arm.body, summary, counters, ctx);
            }
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters, ctx);
            }
        }
        Node::EachBlock(b) => {
            summary.each_block_count += 1;
            walk_fragment(&b.body, summary, counters, ctx);
            if let Some(alt) = &b.alternate {
                walk_fragment(alt, summary, counters, ctx);
            }
        }
        Node::AwaitBlock(b) => {
            if let Some(p) = &b.pending {
                walk_fragment(p, summary, counters, ctx);
            }
            if let Some(t) = &b.then_branch {
                walk_fragment(&t.body, summary, counters, ctx);
            }
            if let Some(c) = &b.catch_branch {
                walk_fragment(&c.body, summary, counters, ctx);
            }
        }
        Node::KeyBlock(b) => walk_fragment(&b.body, summary, counters, ctx),
        Node::SnippetBlock(b) => walk_fragment(&b.body, summary, counters, ctx),
        // Leaf nodes — no children to descend into, no attributes.
        Node::Text(_) | Node::Interpolation(_) | Node::Comment(_) => {}
    }
}

fn walk_attributes(
    attrs: &[Attribute],
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    for attr in attrs {
        if let Attribute::Directive(d) = attr {
            walk_directive(d, summary, counters, ctx);
        }
    }
}

fn walk_directive(
    d: &Directive,
    summary: &mut TemplateSummary,
    counters: &mut Counters,
    ctx: &WalkCtx<'_>,
) {
    match d.kind {
        DirectiveKind::Use => {
            let name = format!("__svn_action_attrs_{}", counters.action_attrs);
            summary.void_refs.register(name);
            counters.action_attrs += 1;
        }
        DirectiveKind::Bind => match &d.value {
            Some(DirectiveValue::BindPair { .. }) => {
                let name = format!("__svn_bind_pair_{}", counters.bind_pair);
                summary.void_refs.register(name);
                counters.bind_pair += 1;
            }
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                // `bind:this={x}` and `bind:foo={x}` (any prop name) — if
                // the bound value is a simple identifier, that local
                // gets assigned asynchronously by Svelte (bind:this when
                // the element mounts; bind:foo when the child component
                // updates the bound prop). Record it for the definite-
                // assignment rewrite so closures reading the variable
                // don't fire TS2454.
                if let Some(name) = simple_identifier_in(ctx.source, *expression_range) {
                    summary.bind_this_targets.push(BindThisTarget {
                        name,
                        range: *expression_range,
                    });
                }
            }
            None => {
                // Bare `bind:foo` is shorthand for `bind:foo={foo}` —
                // same definite-assignment story as the explicit form.
                summary.bind_this_targets.push(BindThisTarget {
                    name: d.name.clone(),
                    range: d.range,
                });
            }
            _ => {}
        },
        _ => {}
    }
}

/// If the byte range covers a single ECMAScript identifier (with optional
/// surrounding whitespace), return it.
fn simple_identifier_in(source: &str, range: Range) -> Option<SmolStr> {
    let slice = source.get(range.start as usize..range.end as usize)?.trim();
    if slice.is_empty() {
        return None;
    }
    let mut chars = slice.chars();
    let first = chars.next()?;
    if !is_ident_start(first) {
        return None;
    }
    if chars.all(is_ident_continue) {
        Some(SmolStr::from(slice))
    } else {
        None
    }
}

#[inline]
fn is_ident_start(c: char) -> bool {
    c.is_ascii_alphabetic() || c == '_' || c == '$'
}

#[inline]
fn is_ident_continue(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_' || c == '$'
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
        walk_template(&fragment, src)
    }

    #[test]
    fn bind_this_simple_identifier_recorded() {
        let s = walk_str("<div bind:this={inputEl} />");
        assert_eq!(s.bind_this_targets.len(), 1);
        assert_eq!(s.bind_this_targets[0].name, "inputEl");
    }

    #[test]
    fn bind_this_complex_expression_not_recorded() {
        // member expressions, calls, etc. shouldn't trigger the rewrite.
        let s = walk_str("<div bind:this={refs[0]} />");
        assert!(s.bind_this_targets.is_empty());
    }

    #[test]
    fn bind_this_with_dollar_identifier_recorded() {
        let s = walk_str("<div bind:this={$el} />");
        assert_eq!(s.bind_this_targets.len(), 1);
        assert_eq!(s.bind_this_targets[0].name, "$el");
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
        // Each `use:foo` directive needs an `__svn_action_attrs_N` holder
        // declared in the template-check function so its inferred attribute
        // type doesn't go unused.
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
        // `bind:foo={getter, setter}` declares a tuple holder; without a
        // void-reference, TypeScript flags it as unused.
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
