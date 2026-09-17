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
//! The "fallback" path for an `<element let:foo>` that's NOT inside a
//! slot is [`emit_children_with_let_bindings`] — emits a loose
//! `{ let foo: any; void foo; …children… }` wrapper so the names
//! resolve as `any`. Type precision is the next iteration's job.

use std::collections::HashMap;
use std::fmt::Write;

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{Fragment, Node};

use crate::emit_buffer::EmitBuffer;
use crate::emit_template_body;
use crate::emit_template_node;
use crate::util::is_simple_js_identifier;

/// One `<Comp let:NAME[={alias|pattern}]>` directive on a component
/// instantiation, captured for the consumer-side slot-def
/// destructure emit (`const { …, NAME } = inst.$$slot_def[…];`).
pub(crate) struct LetDestructure {
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
/// `let:` directives on a regular element (not a component) introduce
/// names into the consumer's scope without a producer-side `slot=`
/// binding. We emit a looser `let name: any;` block so the names
/// resolve inside the subtree. Type precision is lost (the narrower
/// flow-sensitive typing upstream does is the next iteration's job),
/// but TS2304 goes away and any expression referencing the let-name
/// type-checks as `any`.
///
/// If there are no `let:` directives, this is a straight passthrough
/// to `emit_template_body`.
pub(crate) fn emit_children_with_let_bindings(
    buf: &mut EmitBuffer,
    source: &str,
    attributes: &[svn_parser::Attribute],
    children: &Fragment,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let let_names = collect_let_directive_names(source, attributes);
    // When the element ALSO has `slot="X"`, the parent component's
    // child-walk already opened a wrapper destructuring the same
    // let-names against `parent_inst.$$slot_def["X"]` (see
    // `try_emit_slot_let_consumer_open`). Re-emitting `let X: any`
    // shadows here would mask the typed outer destructure — the
    // consumer expressions inside would all resolve to `any` and lose
    // strictness. Pass through to the children walk instead so the
    // outer destructure stays in scope.
    let parent_destructured = svn_analyze::literal_attr_value(attributes, "slot", source).is_some();
    if let_names.is_empty() || parent_destructured {
        emit_template_body(buf, source, children, depth, insts, action_counter);
        return;
    }
    let indent = "    ".repeat(depth);
    let inner = "    ".repeat(depth + 1);
    let _ = writeln!(buf, "{indent}{{");
    for name in &let_names {
        let _ = writeln!(buf, "{inner}let {name}: any;");
        let _ = writeln!(buf, "{inner}void {name};");
    }
    for node in &children.nodes {
        emit_template_node(buf, source, node, depth + 1, insts, action_counter);
    }
    let _ = writeln!(buf, "{indent}}}");
}

/// Extract every binding name introduced by `let:X` directives on
/// `attributes`. Handles both shorthand (`let:item` → "item") and
/// aliased form (`let:item={i}` → "i"). Non-identifier destructure
/// patterns (`let:item={{a, b}}`) aren't narrowed — we take the
/// original directive name as the binding instead, which is a
/// harmless no-op but avoids parse-ambiguity.
fn collect_let_directive_names(source: &str, attributes: &[svn_parser::Attribute]) -> Vec<SmolStr> {
    use svn_parser::{Attribute, Directive, DirectiveKind, DirectiveValue};
    let mut out: Vec<SmolStr> = Vec::new();
    for attr in attributes {
        if let Attribute::Directive(Directive {
            kind: DirectiveKind::Let,
            name,
            value,
            ..
        }) = attr
        {
            let bound = match value {
                Some(DirectiveValue::Expression {
                    expression_range, ..
                }) => {
                    let start = expression_range.start as usize;
                    let end = expression_range.end as usize;
                    let slice = source.get(start..end).unwrap_or("").trim();
                    if is_simple_js_identifier(slice) {
                        SmolStr::from(slice)
                    } else {
                        name.clone()
                    }
                }
                _ => name.clone(),
            };
            if !out.iter().any(|n| n == &bound) {
                out.push(bound);
            }
        }
    }
    out
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
    for attr in attributes {
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
        buf.append_with_source(&d.pattern_text[..d.name_byte_len], d.name_range);
        if d.name_byte_len < d.pattern_text.len() {
            buf.push_str(&d.pattern_text[d.name_byte_len..]);
        }
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

/// True when `node` is a child element carrying `slot="X"` — a slot
/// consumer of its parent component. Used to pre-flag the parent so its
/// instance gets hoisted to a local (the wrapper destructure references
/// `parent_inst.$$slot_def["X"]`).
pub(crate) fn child_is_slot_let_consumer(source: &str, node: &Node) -> bool {
    let Some(attrs) = slot_let_attrs(node) else {
        return false;
    };
    svn_analyze::literal_attr_value(attrs, "slot", source).is_some()
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
        return false;
    };
    let lets = collect_let_destructures(source, attrs);
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}{{");
    emit_let_slot_destructure(
        buf,
        parent_inst,
        &lets,
        slot_name,
        Some((node.range().start, name_range)),
        depth + 1,
    );
    true
}

#[inline]
fn emit_slot_let_consumer_close(buf: &mut EmitBuffer, depth: usize) {
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}}}");
}

/// Walk one child of a component, opening a slot-let consumer wrapper
/// first when the child is a `<Inner slot="X" let:foo>` pattern.
/// Bumps the walk depth by one inside the wrapper so the child's own
/// emit nests under the destructure.
pub(crate) fn walk_child_with_slot_let(
    buf: &mut EmitBuffer,
    source: &str,
    node: &Node,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
    parent_inst: Option<&svn_analyze::ComponentInstantiation>,
) {
    let opened = parent_inst
        .map(|p| try_emit_slot_let_consumer_open(buf, source, node, p, depth))
        .unwrap_or(false);
    let walk_depth = if opened { depth + 1 } else { depth };
    emit_template_node(buf, source, node, walk_depth, insts, action_counter);
    if opened {
        emit_slot_let_consumer_close(buf, depth);
    }
}
