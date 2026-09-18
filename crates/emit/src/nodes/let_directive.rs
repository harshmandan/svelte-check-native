//! `let:NAME[={alias|pattern}]` slot-let directive emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/Let.ts`.
//!
//! Two consumer-side patterns are handled:
//!
//! 1. `let:` on the component itself — destructure against
//!    `inst.$$slot_def.default` inside the component-call's inner
//!    block. Driven from [`crate::nodes::inline_component::emit_component_node`].
//! 2. `let:` on a child element/component carrying `slot="X"` — wrap
//!    the child at the parent's child-walk depth and destructure
//!    against `parent_inst.$$slot_def["X"]`. Driven from
//!    [`walk_child_with_slot_let`].
//!
//! 3. `let:` on a child element (or `<svelte:fragment>`) with no
//!    `slot=` — it fills the parent's default slot, so the same wrapper
//!    destructures against `parent_inst.$$slot_def.default`.
//!
//! Any other `<element let:foo>` gets no declaration: svelte2tsx writes
//! the directive as an ordinary attribute (see
//! [`emit_children_with_let_bindings`]).

use std::collections::HashMap;
use std::fmt::Write;

use svn_core::Range;
use svn_parser::{Fragment, Node};

use crate::emit_buffer::EmitBuffer;

thread_local! {
    /// For each template position being walked, the instance of the
    /// component a slot-filling element there belongs to (its
    /// `node_start`), or `None` inside an element or snippet.
    /// svelte2tsx pairs an element with the innermost enclosing element
    /// or component, looking through `{#if}` / `{#each}` / `{#await}` /
    /// `{#key}` blocks, so blocks leave this untouched.
    static SLOT_PARENT: std::cell::RefCell<Vec<Option<u32>>> =
        const { std::cell::RefCell::new(Vec::new()) };
    /// Start offsets of the first `let:` directive of each element whose
    /// `let:` names a parent component already destructured from its
    /// default slot. The element's own emit must not shadow them.
    static PARENT_DESTRUCTURED_LETS: std::cell::RefCell<Vec<u32>> =
        const { std::cell::RefCell::new(Vec::new()) };
}

/// Start offset of the first `let:` directive among `attributes`.
fn first_let_start(attributes: &[svn_parser::Attribute]) -> Option<u32> {
    attributes.iter().find_map(|a| match a {
        svn_parser::Attribute::Directive(d) if d.kind == svn_parser::DirectiveKind::Let => {
            Some(d.range.start)
        }
        _ => None,
    })
}
use crate::emit_template_body;
use crate::emit_template_node;

/// One `<Comp let:NAME[={alias|pattern}]>` directive on a component
/// instantiation, captured for the consumer-side slot-def
/// destructure emit (`const { …, NAME } = inst.$$slot_def[…];`).
pub(crate) struct LetDestructure {
    /// In-tag comments written around the `let:` directive.
    comments: svn_analyze::CommentThread,
    /// Length in bytes of the leading NAME portion of `pattern_text`
    /// — anchor for the TokenMap entry that maps the NAME bytes in
    /// the destructure literal back to the source `let:NAME`
    /// position. Without it, tsgo diagnostics on the destructure
    /// entry (TS2339 "Property 'foo' does not exist on type
    /// 'Slots[X]'") fall through `translate_position` and get
    /// dropped.
    name_byte_len: usize,
    /// Source byte range of NAME in the original `let:NAME` directive
    /// — anchor for the TokenMap entry described above. Targets the
    /// NAME bytes specifically (skips the `let:` prefix), so a
    /// diagnostic on the destructure literal maps to the source
    /// NAME the user wrote.
    name_range: Range,
    /// Source position a diagnostic on text written after this item
    /// maps to: the brace closing its `={…}` value, or the last
    /// character of NAME when it has none.
    tail_anchor: Range,
    /// Source slice for the destructure pattern. For bare `let:foo`
    /// this is `"foo"`; for `let:foo={alias}` it's `"foo: alias"`;
    /// for destructure `let:foo={{a, b}}` it's `"foo: {a, b}"`.
    /// Spliced verbatim into the destructure literal — the leading
    /// `name_byte_len` bytes get a TokenMap entry, the rest is plain.
    pattern_text: String,
}

/// Walk the children of an element that carries `let:NAME` directives.
///
/// When a parent component destructured the names (see
/// [`walk_child_with_slot_let`]) they are already in scope. Otherwise
/// svelte2tsx writes each `let:` as an ordinary attribute
/// (`Let.ts` → `handleAttribute`) and declares nothing, so the names
/// stay undeclared in the children as well.
pub(crate) fn emit_children_with_let_bindings(
    buf: &mut EmitBuffer,
    source: &str,
    _attributes: &[svn_parser::Attribute],
    children: &Fragment,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    emit_template_body(buf, source, children, depth, insts, action_counter);
}

/// Were the `let:` directives among `attributes` destructured by the
/// enclosing component's child walk? Otherwise they are attributes.
pub(crate) fn lets_destructured_by_parent(attributes: &[svn_parser::Attribute]) -> bool {
    first_let_start(attributes)
        .is_some_and(|start| PARENT_DESTRUCTURED_LETS.with(|set| set.borrow().contains(&start)))
}

/// Build the `LetDestructure` list for one let-bearing element/component.
/// Each `let:` directive becomes one entry in the consumer-side
/// destructure literal. The slot name (default vs `slot="X"`) is the
/// caller's concern — the same list is destructured against either
/// `inst.$$slot_def.default` (let on the component itself) or
/// `parent.$$slot_def["X"]` (let on a `slot="X"` child).
pub(crate) fn collect_let_destructures(
    source: &str,
    attributes: &[svn_parser::Attribute],
) -> Vec<LetDestructure> {
    use svn_parser::{Attribute, Directive, DirectiveKind, DirectiveValue};
    let mut out: Vec<LetDestructure> = Vec::new();
    for (index, attr) in attributes.iter().enumerate() {
        let Attribute::Directive(d) = attr else {
            continue;
        };
        let Directive {
            kind: DirectiveKind::Let,
            name,
            value,
            range,
            ..
        } = d
        else {
            continue;
        };
        let pattern_text = match value {
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                let start = expression_range.start as usize;
                let end = expression_range.end as usize;
                let slice = source.get(start..end).unwrap_or("").trim();
                if slice.is_empty() || slice == name.as_str() {
                    // `let:foo` / `let:foo={foo}` — shorthand `foo`.
                    name.to_string()
                } else {
                    // `let:foo={alias}` or `let:foo={{a, b}}`.
                    format!("{}: {}", name.as_str(), slice)
                }
            }
            _ => name.to_string(),
        };
        let name_start = range.start + DirectiveKind::Let.prefix_len_with_colon();
        let name_end = name_start + name.len() as u32;
        // Upstream moves `NAME` (and `:EXPR`) into the destructure and
        // writes its own text after it; its source map resolves that
        // text to the closing brace after an expression, or to the
        // name's last character.
        let tail_anchor = match value {
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                let raw = source
                    .get(expression_range.start as usize..expression_range.end as usize)
                    .unwrap_or("");
                let end = expression_range.end - (raw.len() - raw.trim_end().len()) as u32;
                Range::new(end, end + 1)
            }
            _ => Range::new(name_end.saturating_sub(1), name_end),
        };
        out.push(LetDestructure {
            comments: svn_analyze::comment_thread(attributes, index, source),
            name_byte_len: name.len(),
            name_range: Range::new(name_start, name_end),
            tail_anchor,
            pattern_text,
        });
    }
    out
}

/// Emit the consumer-side `const { $$_$$, foo, bar } =
/// __svn_inst_<hex>.$$slot_def.<slotName>; $$_$$;` line(s) inside
/// the component-call block. One line per slot referenced (for
/// default-only consumers, exactly one line). Mirrors upstream
/// svelte2tsx's InlineComponent.ts:184-207.
///
/// The `$$_$$` dummy + immediate void usage is upstream's trick to
/// suppress TS6133 ("declared but never read") on the whole
/// destructure list when all let-bindings happen to be unused.
/// The `/*Ωignore_startΩ*/.../*Ωignore_endΩ*/` wrapper around the dummy
/// name is emit-shape parity with upstream svelte2tsx and nothing more.
/// It does NOT suppress anything: our diagnostic mapper scans for the
/// ASCII `IGNORE_START_MARKER` pair, not this one. What keeps a
/// diagnostic on `$$_$$` from surfacing is that the position has no
/// token-map entry, so the mapper drops it as generated code.
///
/// `slot_anchor` is set for a `slot="X"` child: the start of that child
/// and the range of the `X` text. A diagnostic on `$$slot_def["X"]`
/// (an unknown slot name) then maps from the child's start to the end
/// of the name, as upstream's does.
pub(crate) fn emit_let_slot_destructure(
    buf: &mut EmitBuffer,
    source: &str,
    inst: &svn_analyze::ComponentInstantiation,
    let_destructures: &[LetDestructure],
    slot_name: &str,
    slot_anchor: Option<(u32, svn_core::Range)>,
    depth: usize,
) {
    let inst_local = svn_core::synth_names::instance_local(inst.node_start);
    let indent = "    ".repeat(depth);
    // Upstream's `$$_$$` dummy keeps TS6133 quiet on unused
    // destructure lists; the omega markers are decorative (matched
    // upstream svelte2tsx's marker style for symmetry — our scanner
    // tolerates the missing TokenMap on `$$_$$` because the
    // diagnostic mapper drops unmapped synthesised positions
    // anyway).
    let _ = write!(
        buf,
        "{indent}const {{ /*\u{03A9}ignore_start\u{03A9}*/$$_$$/*\u{03A9}ignore_end\u{03A9}*/"
    );
    for d in let_destructures {
        buf.push_str(", ");
        // Push a TokenMap entry for the NAME bytes so a tsgo
        // diagnostic on the destructure (TS2339 "Property 'X' does
        // not exist on type 'Slots[Y]'") survives `translate_position`
        // and maps back to the source `let:NAME` position. Without
        // this, the diagnostic falls inside the synthesised
        // destructure literal, finds no token-map cover, and is
        // dropped — leaving a real divergence with upstream
        // (e.g. slot-typechecks fixture's TS2339 on `let:d` against
        // a slot typed `{a: boolean, b: string}`).
        crate::nodes::comment::write_leading_comments(buf, source, &d.comments);
        buf.append_with_source(&d.pattern_text[..d.name_byte_len], d.name_range);
        if d.name_byte_len < d.pattern_text.len() {
            buf.push_str(&d.pattern_text[d.name_byte_len..]);
        }
        crate::nodes::comment::write_trailing_comments(buf, source, &d.comments);
    }
    match slot_anchor {
        Some((child_start, name_range)) => {
            // Upstream inserts the quotes around the moved slot name,
            // so a diagnostic reported at the opening quote lands on
            // the child's start and one at the closing quote on the
            // name's end.
            let child = svn_core::Range::new(child_start, child_start + 1);
            let after_name = svn_core::Range::new(name_range.end, name_range.end + 1);
            buf.push_str(" } = ");
            buf.append_with_source(&format!("{inst_local}.$$slot_def["), child);
            buf.append_with_source("\"", child);
            buf.append_with_source(slot_name, name_range);
            buf.append_with_source("\"", after_name);
            buf.push_str("]; $$_$$;\n");
        }
        None if slot_name == "default" => {
            // A diagnostic on the slot access (a component whose slots
            // are `unknown`) lands where upstream's does: after the
            // last `let:` item.
            buf.push_str(" } = ");
            let access = format!("{inst_local}.$$slot_def.default");
            match let_destructures.last() {
                Some(last) => buf.append_with_source(&access, last.tail_anchor),
                None => buf.push_str(&access),
            }
            buf.push_str("; $$_$$;\n");
        }
        None => {
            let _ = writeln!(
                buf,
                " }} = {inst_local}.$$slot_def[\"{slot_name}\"]; $$_$$;"
            );
        }
    }
}

/// Pluck the slot-let-consumer attributes off any node shape that can
/// legally carry both `slot="X"` and `let:Y` — components, regular
/// DOM elements (`<div slot="X" let:foo>`), and special elements
/// (`<svelte:fragment slot="X" let:foo>`). Returns `None` for nodes
/// that aren't elements at all (text, blocks, etc).
fn slot_let_attrs(node: &Node) -> Option<&[svn_parser::Attribute]> {
    match node {
        Node::Component(c) => Some(c.attributes.as_slice()),
        Node::Element(e) => Some(e.attributes.as_slice()),
        Node::SvelteElement(e) => Some(e.attributes.as_slice()),
        _ => None,
    }
}

/// True when `node` is a slot consumer of its parent component: a child
/// carrying `slot="X"`, or an element / `<svelte:fragment>` whose `let:`
/// reads the default slot. Used to pre-flag the parent so its instance
/// gets hoisted to a local (the wrapper destructure references
/// `parent_inst.$$slot_def`).
pub(crate) fn child_is_slot_let_consumer(source: &str, node: &Node) -> bool {
    let Some(attrs) = slot_let_attrs(node) else {
        return false;
    };
    svn_analyze::literal_attr_value(attrs, "slot", source).is_some()
        || (fills_parent_default_slot(node) && first_let_start(attrs).is_some())
}

/// Whether `let:` on `node` (with no `slot=`) reads its parent
/// component's default slot rather than a slot of its own.
fn fills_parent_default_slot(node: &Node) -> bool {
    match node {
        Node::Element(_) => true,
        Node::SvelteElement(e) => !matches!(
            e.kind,
            svn_parser::SvelteElementKind::SelfRef | svn_parser::SvelteElementKind::Component
        ),
        _ => false,
    }
}

/// If `node` is a child element carrying `slot="X"`, open a wrapper
/// block at the parent's child-walk depth and emit the consumer-side
/// destructure against `parent_inst.$$slot_def["X"]` — with the
/// child's `let:` names, or just upstream's `$$_$$` dummy when it has
/// none, so a slot name the component does not declare is reported
/// either way. Returns `true` when the wrapper was opened — caller
/// closes it via `emit_slot_let_consumer_close` after walking the child.
///
/// Mirrors upstream svelte2tsx's `Attribute.ts` (`addSlotName` for
/// every `slot=` child of a component) and InlineComponent.ts:184-207,
/// where the destructure lives in the OUTER component's block — so the
/// let names are in scope across the inner emit.
///
/// Accepts component, DOM-element, and `<svelte:fragment>` children.
fn try_emit_slot_let_consumer_open(
    buf: &mut EmitBuffer,
    source: &str,
    node: &Node,
    parent_inst: &svn_analyze::ComponentInstantiation,
    depth: usize,
) -> bool {
    let Some(attrs) = slot_let_attrs(node) else {
        return false;
    };
    let Some((slot_name, name_range)) =
        svn_analyze::literal_attr_value_range(attrs, "slot", source)
    else {
        return try_emit_default_slot_let_open(buf, source, node, attrs, parent_inst, depth);
    };
    let lets = collect_let_destructures(source, attrs);
    if let Some(start) = first_let_start(attrs) {
        PARENT_DESTRUCTURED_LETS.with(|set| set.borrow_mut().push(start));
    }
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}{{");
    emit_let_slot_destructure(
        buf,
        source,
        parent_inst,
        &lets,
        slot_name,
        Some((node.range().start, name_range)),
        depth + 1,
    );
    true
}

/// An element (or `<svelte:fragment>`) with `let:` directives and no
/// `slot=` fills its parent component's default slot, so svelte2tsx
/// destructures its `let:` names from the parent's
/// `$$slot_def.default` (`Element.ts` slot-let transformation). A
/// component child is excluded: its `let:` reads its own default slot.
fn try_emit_default_slot_let_open(
    buf: &mut EmitBuffer,
    source: &str,
    node: &Node,
    attrs: &[svn_parser::Attribute],
    parent_inst: &svn_analyze::ComponentInstantiation,
    depth: usize,
) -> bool {
    let Some(first_let) = first_let_start(attrs) else {
        return false;
    };
    if !fills_parent_default_slot(node) {
        return false;
    }
    let lets = collect_let_destructures(source, attrs);
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}{{");
    emit_let_slot_destructure(buf, source, parent_inst, &lets, "default", None, depth + 1);
    PARENT_DESTRUCTURED_LETS.with(|set| set.borrow_mut().push(first_let));
    true
}

#[inline]
fn emit_slot_let_consumer_close(buf: &mut EmitBuffer, depth: usize) {
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}}}");
}

/// Walk one child of a component. Elements it (or a block inside it)
/// contains that fill one of the component's slots are wrapped by
/// [`emit_slot_parented_node`].
pub(crate) fn walk_child_with_slot_let(
    buf: &mut EmitBuffer,
    source: &str,
    node: &Node,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
    parent_inst: Option<&svn_analyze::ComponentInstantiation>,
) {
    with_slot_parent(parent_inst.map(|p| p.node_start), || {
        emit_template_node(buf, source, node, depth, insts, action_counter);
    });
}

/// Run `f` with `parent` as the slot parent of the nodes it walks.
pub(crate) fn with_slot_parent<R>(parent: Option<u32>, f: impl FnOnce() -> R) -> R {
    SLOT_PARENT.with(|s| s.borrow_mut().push(parent));
    let out = f();
    SLOT_PARENT.with(|s| s.borrow_mut().pop());
    out
}

/// Emit an element-like node (element, component, special element),
/// first opening a slot-let consumer wrapper when it fills a slot of
/// the enclosing component (`<Inner slot="X" let:foo>`, or an element
/// with `let:` filling the default slot). The node's own children get
/// no slot parent unless it is a component and sets its own.
pub(crate) fn emit_slot_parented_node(
    buf: &mut EmitBuffer,
    source: &str,
    node: &Node,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    emit: impl FnOnce(&mut EmitBuffer, usize),
) {
    let parent = SLOT_PARENT
        .with(|s| s.borrow().last().copied().flatten())
        .and_then(|key| insts.get(&key).copied());
    let opened = parent
        .map(|p| try_emit_slot_let_consumer_open(buf, source, node, p, depth))
        .unwrap_or(false);
    let walk_depth = if opened { depth + 1 } else { depth };
    with_slot_parent(None, || emit(buf, walk_depth));
    if opened {
        emit_slot_let_consumer_close(buf, depth);
    }
}

/// Does a component's content hold a slot consumer of it — directly
/// or inside blocks, but not inside elements, components or snippets?
pub(crate) fn fragment_has_slot_let_consumer(source: &str, fragment: &Fragment) -> bool {
    fragment.nodes.iter().any(|n| match n {
        Node::IfBlock(b) => {
            fragment_has_slot_let_consumer(source, &b.consequent)
                || b.elseif_arms
                    .iter()
                    .any(|arm| fragment_has_slot_let_consumer(source, &arm.body))
                || b.alternate
                    .as_ref()
                    .is_some_and(|f| fragment_has_slot_let_consumer(source, f))
        }
        Node::EachBlock(b) => {
            fragment_has_slot_let_consumer(source, &b.body)
                || b.alternate
                    .as_ref()
                    .is_some_and(|f| fragment_has_slot_let_consumer(source, f))
        }
        Node::AwaitBlock(b) => {
            b.pending
                .as_ref()
                .is_some_and(|f| fragment_has_slot_let_consumer(source, f))
                || b.then_branch
                    .as_ref()
                    .is_some_and(|t| fragment_has_slot_let_consumer(source, &t.body))
                || b.catch_branch
                    .as_ref()
                    .is_some_and(|c| fragment_has_slot_let_consumer(source, &c.body))
        }
        Node::KeyBlock(b) => fragment_has_slot_let_consumer(source, &b.body),
        other => child_is_slot_let_consumer(source, other),
    })
}
