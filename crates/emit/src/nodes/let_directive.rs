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
    /// Source slice for the destructure pattern. For bare `let:foo`
    /// this is `"foo"`; for `let:foo={alias}` it's `"foo: alias"`;
    /// for destructure `let:foo={{a, b}}` it's `"foo: {a, b}"`.
    /// Spliced verbatim into the destructure literal — the leading
    /// `name_byte_len` bytes get a TokenMap entry, the rest is plain.
    pattern_text: String,
    /// The local binding the destructure introduces — `name` for
    /// shorthand, the alias for `let:foo={alias}`. None for
    /// non-identifier nested patterns (`let:foo={{a, b}}`); those
    /// don't get a `void <X>;` suppressor since the inner names
    /// resolve in the body's natural scope.
    void_target: Option<SmolStr>,
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
    let parent_destructured = svn_analyze::literal_attr_value(attributes, "slot").is_some();
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
        let (pattern_text, void_target): (String, Option<SmolStr>) = match value {
            Some(DirectiveValue::Expression {
                expression_range, ..
            }) => {
                let start = expression_range.start as usize;
                let end = expression_range.end as usize;
                let slice = source.get(start..end).unwrap_or("").trim();
                if slice.is_empty() {
                    (name.to_string(), Some(name.clone()))
                } else if is_simple_js_identifier(slice) && slice == name.as_str() {
                    // `let:foo={foo}` — same name on both sides;
                    // emit shorthand `foo`.
                    (name.to_string(), Some(name.clone()))
                } else if is_simple_js_identifier(slice) {
                    // `let:foo={alias}` — alias rename. The introduced
                    // local is the alias; `void <alias>;` suppresses
                    // TS6133 on it.
                    (
                        format!("{}: {}", name.as_str(), slice),
                        Some(SmolStr::from(slice)),
                    )
                } else {
                    // `let:foo={{a, b}}` — nested pattern. Emit as
                    // `foo: <pattern>`. No `void` target — the inner
                    // names resolve in the body's natural scope.
                    (format!("{}: {}", name.as_str(), slice), None)
                }
            }
            _ => (name.to_string(), Some(name.clone())),
        };
        let name_start = range.start + DirectiveKind::Let.prefix_len_with_colon();
        let name_end = name_start + name.len() as u32;
        out.push(LetDestructure {
            name_byte_len: name.len(),
            name_range: Range::new(name_start, name_end),
            pattern_text,
            void_target,
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
/// Wrapping the dummy name in `/*Ωignore_startΩ*/.../*Ωignore_endΩ*/`
/// markers keeps any source-position diagnostic on it from
/// surfacing.
pub(crate) fn emit_let_slot_destructure(
    buf: &mut EmitBuffer,
    inst: &svn_analyze::ComponentInstantiation,
    let_destructures: &[LetDestructure],
    slot_name: &str,
    depth: usize,
) {
    if let_destructures.is_empty() {
        return;
    }
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
    let _ = if slot_name == "default" {
        writeln!(buf, " }} = {inst_local}.$$slot_def.default; $$_$$;")
    } else {
        writeln!(buf, " }} = {inst_local}.$$slot_def[\"{slot_name}\"]; $$_$$;")
    };
    // `void <name>;` per let-binding suppresses TS6133 on names the
    // user's slot body doesn't reference. Without this the new
    // TokenMap entry on the destructure name surfaces 6133 at the
    // source `let:NAME` position — a regression for slot-let
    // wrappers that forward without consuming (canonical layerchart
    // pattern: `<Wrapper let:tooltip><slot {tooltip} /></Wrapper>`).
    // 2339 / 2367 / 2353 on the destructure entry still fire
    // because they target the destructure pattern itself, not the
    // local binding's later use.
    for d in let_destructures {
        if let Some(target) = &d.void_target {
            let _ = writeln!(buf, "{indent}void {target};");
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

/// True when `node` is a child element carrying both `slot="X"` and at
/// least one `let:` directive — a slot-let consumer of its parent
/// component. Used to pre-flag the parent so its instance gets hoisted
/// to a local (the wrapper destructure references
/// `parent_inst.$$slot_def["X"]`).
pub(crate) fn child_is_slot_let_consumer(source: &str, node: &Node) -> bool {
    let Some(attrs) = slot_let_attrs(node) else {
        return false;
    };
    if svn_analyze::literal_attr_value(attrs, "slot").is_none() {
        return false;
    }
    !collect_let_destructures(source, attrs).is_empty()
}

/// If `node` is a child element carrying both `slot="X"` and one or
/// more `let:` directives, open a wrapper block at the parent's
/// child-walk depth and emit the consumer-side destructure against
/// `parent_inst.$$slot_def["X"]`. Returns `true` when the wrapper was
/// opened — caller closes it via `emit_slot_let_consumer_close` after
/// walking the child.
///
/// Mirrors upstream svelte2tsx's InlineComponent.ts:184-207, where the
/// destructure for `<Inner slot="X" let:foo>` lives in the OUTER
/// component's block — so `foo` is in scope across the inner emit
/// (notably the inner component-call's `$on(...)` handler that
/// references `foo`, which sits before the inner's own children walk).
///
/// Accepts component, DOM-element, and `<svelte:fragment>` children.
/// All three carry `slot=` + `let:` legally; the wrap mechanics are
/// identical regardless of which element kind houses the directives.
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
    let Some(slot_name) = svn_analyze::literal_attr_value(attrs, "slot") else {
        return false;
    };
    let lets = collect_let_destructures(source, attrs);
    if lets.is_empty() {
        return false;
    }
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}{{");
    emit_let_slot_destructure(buf, parent_inst, &lets, slot_name, depth + 1);
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
