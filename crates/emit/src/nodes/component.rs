//! Component-call emission and the prop-writer / snippet-arrow helpers
//! it shares with adjacent emitters (e.g. `nodes::blocks`).
//!
//! Each `<Comp …>` becomes a scoped `{ const $$_C = …; new $$_C({…}); }`
//! block. The wrapper + local + `new` form lets generic components'
//! `<T>` resolve at the `new` site against concrete prop values; the
//! intermediate local is load-bearing (see CLAUDE.md and design/phase_a/).

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::{Node, SnippetBlock};

use crate::TokenMapEntry;
use crate::emit_buffer::EmitBuffer;
use crate::util::{is_css_custom_prop_name, is_simple_js_identifier};
use crate::{
    all_identifiers, child_is_slot_let_consumer, collect_let_destructures,
    emit_let_slot_destructure, emit_template_body, walk_child_with_slot_let,
};

/// Emit a `<Component ...>` node as a call to the component's typed
/// default export:
///
/// ```ts
///     ComponentRoot(__svn_any(), {
///         prop1: "lit",
///         prop2: (expr),
///         prop3,
///         prop4: true,
///         snippetName: (params) => {
///             <snippet body>
///             return __svn_snippet_return();
///         },
///     });
/// ```
///
/// The callable shape is what makes TypeScript's contextual typing
/// work here. The component's `.svelte.ts` overlay exports a
/// `declare function __svn_component_default(anchor, props: Props)`;
/// each prop slot's declared type flows into the consumer's expression
/// at this call site.
///
/// Directive-attached props (`bind:value`, `on:click`, `use:action`,
/// `class:active`, etc.) and spreads are skipped at analyze time.
/// `__svn_any()` returns a fresh `any`; the first argument slot
/// simulates Svelte's mount-anchor parameter without constraining
/// inference.
///
/// Non-snippet children (text, elements, nested components, blocks) are
/// walked AFTER the call-site scaffolding in the component's own scope.
/// Direct-child `{#snippet name(params)}` blocks ARE consumed as props
/// on the object literal — not walked a second time — so their param
/// destructures pick up contextual types from the parent's Snippet prop
/// shape instead of reading as implicit-any.
pub(crate) fn emit_component_node(
    buf: &mut EmitBuffer,
    source: &str,
    c: &svn_parser::Component,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let snippet_children: Vec<&SnippetBlock> = c
        .children
        .nodes
        .iter()
        .filter_map(|n| match n {
            Node::SnippetBlock(b) => Some(b),
            _ => None,
        })
        .collect();

    let inst = insts.get(&c.range.start);
    // SVELTE-4-COMPAT: `<Comp let:foo>` consumer-side bindings.
    //
    // `let:` directives on a component WITHOUT `slot="X"` consume the
    // component's own default slot — destructured against
    // `inst.$$slot_def.default` inside its inner block. Mirrors
    // upstream svelte2tsx's InlineComponent.ts:184-207 destructure shape.
    //
    // `let:` directives on a component WITH `slot="X"` are slot-CONSUMER
    // bindings: they pull from the PARENT component's `$$slot_def["X"]`
    // and the destructure must wrap this child at the parent's child-
    // walk depth (handled in the parent's loop via
    // `try_emit_slot_let_consumer_open`). Skipping here prevents a
    // double destructure / wrong access path.
    let has_slot_attr = svn_analyze::literal_attr_value(&c.attributes, "slot").is_some();
    let let_destructures = if has_slot_attr {
        Vec::new()
    } else {
        collect_let_destructures(source, &c.attributes)
    };
    let has_let_bindings = !let_destructures.is_empty();
    // Pre-scan children for the `<Inner slot="X" let:foo>` shape — when
    // present, the parent (this component) needs its instance hoisted
    // to a local so the consumer wrapper can reference
    // `parent.$$slot_def["X"]`.
    let any_child_consumes_slot_let = c
        .children
        .nodes
        .iter()
        .any(|n| child_is_slot_let_consumer(source, n));

    // Only emit the call when analyze collected an instantiation for
    // this node. Components disqualified at analyze time fall back to
    // a plain template-body walk so snippet hoists still emit there.
    //
    // The call leaves its `{ … }` block OPEN. Children walk INSIDE the
    // block — `{@const X = …}` declarations therefore live in the
    // component's block scope, so sibling components each have their
    // own X without colliding (TS2451 redeclare). Caller closes via
    // `emit_component_call_close` after the walk.
    let opened_call_block = inst.is_some();
    let child_depth = if opened_call_block { depth + 1 } else { depth };
    if let Some(inst) = inst {
        emit_component_call(
            buf,
            source,
            inst,
            depth,
            &snippet_children,
            insts,
            action_counter,
            has_let_bindings || any_child_consumes_slot_let,
        );
    }

    // Slot-let destructure goes in an INNER block so the user-source
    // names declared via `let:foo` shadow only inside the children
    // walk. The component-call's `new __svn_C({props: {foo: foo}})`
    // references at the OUTER block resolve to module-scope `foo`
    // (avoids TDZ on consumers like layerchart Chart.svelte's
    // `<LayerCake yScale={yScale} let:yScale>` where the let-name
    // shadows a module-scope export of the same name).
    let inner_block_for_let = has_let_bindings;
    let final_child_depth = if inner_block_for_let {
        let inner_open_indent = "    ".repeat(child_depth);
        let _ = writeln!(buf, "{inner_open_indent}{{");
        let dest_depth = child_depth + 1;
        if let Some(inst) = inst {
            emit_let_slot_destructure(buf, inst, &let_destructures, "default", dest_depth);
        }
        dest_depth
    } else {
        child_depth
    };

    if inst.is_none() || snippet_children.is_empty() {
        for node in &c.children.nodes {
            walk_child_with_slot_let(
                buf,
                source,
                node,
                final_child_depth,
                insts,
                action_counter,
                inst.copied(),
            );
        }
        if inner_block_for_let {
            let inner_close_indent = "    ".repeat(child_depth);
            let _ = writeln!(buf, "{inner_close_indent}}}");
        }
        if opened_call_block {
            emit_component_call_close(buf, depth);
        }
        return;
    }
    // Snippet children consumed as props above — walk the rest.
    for node in &c.children.nodes {
        if matches!(node, Node::SnippetBlock(_)) {
            continue;
        }
        walk_child_with_slot_let(
            buf,
            source,
            node,
            final_child_depth,
            insts,
            action_counter,
            inst.copied(),
        );
    }
    if inner_block_for_let {
        let inner_close_indent = "    ".repeat(child_depth);
        let _ = writeln!(buf, "{inner_close_indent}}}");
    }
    if opened_call_block {
        emit_component_call_close(buf, depth);
    }
}

/// Emit the call-site scaffolding for one component instantiation as
///
/// ```text
/// { const $$_CN = __svn_ensure_component(Comp);
///   new $$_CN({ target: __svn_any(), props: { ... } }); }
/// ```
///
/// — the wrapper + local + `new` form chosen so a single emission
/// handles both our overlay-declared class defaults and third-party
/// `extends SvelteComponent` classes (and bare `Component<Props>`
/// values from user-typed contexts). The intermediate local is what
/// makes generic components' `<T>` resolve at the `new` site — TS
/// binds the construct signature's generics against the concrete prop
/// values there, not at the `__svn_ensure_component` site.
///
/// Each instantiation gets its own block scope `{ ... }` so the
/// synthesized `$$_CN` local is siloed from sibling instantiations —
/// avoids shadowing / redeclaration when the same parent fragment
/// contains multiple components.
#[allow(clippy::too_many_arguments)]
fn emit_component_call(
    buf: &mut EmitBuffer,
    source: &str,
    inst: &svn_analyze::ComponentInstantiation,
    depth: usize,
    snippet_children: &[&SnippetBlock],
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
    needs_inst_for_let: bool,
) {
    let indent = "    ".repeat(depth);
    let inner = "    ".repeat(depth + 1);
    let comp = &inst.component_root;
    // Per-call-site synthesized locals. Hoisted into a named local
    // when a post-construction emit needs it (`$inst.$on(...)`,
    // `bind:this`, slot-let consumer destructure).
    let local = svn_core::synth_names::component_local(inst.node_start);
    let inst_local = svn_core::synth_names::instance_local(inst.node_start);
    let hoist_instance =
        !inst.on_events.is_empty() || inst.bind_this_target.is_some() || needs_inst_for_let;
    let ctor_lhs = if hoist_instance {
        format!("const {inst_local} = ")
    } else {
        String::new()
    };

    let _ = writeln!(buf, "{indent}{{");
    let _ = writeln!(
        buf,
        "{inner}const {local} = __svn_ensure_component({comp});"
    );

    // Implicit-children synthesis: when the user has non-snippet body
    // content, inject `"children": () => __svn_snippet_return()` into
    // the props literal. Matches upstream svelte2tsx's behavior so
    // components declaring `children: Snippet` (required) accept
    // `<Comp>body</Comp>` without a TS2741 at the satisfies trailer.
    //
    // Skipped when the user explicitly named a `children` prop OR
    // wrote a `{#snippet children}` block — both paths already emit
    // a `children:` key; a second synthesis would fire TS1117.
    let user_named_children = inst
        .props
        .iter()
        .any(|p| prop_shape_name(p).is_some_and(|n| n == "children"));
    let user_named_children_snippet = snippet_children
        .iter()
        .any(|s| s.name.as_str() == "children");
    let emit_implicit_children =
        inst.has_implicit_children && !user_named_children && !user_named_children_snippet;

    if snippet_children.is_empty() && inst.props.is_empty() && !emit_implicit_children {
        let _ = write!(buf, "{inner}{ctor_lhs}");
        let call_start = buf.len() as u32;
        let _ = write!(buf, "new {local}({{ target: __svn_any(), props: {{}} }})");
        push_component_call_token_map(buf, call_start, inst.node_start);
        buf.push_str(";\n");
        emit_component_bind_widen_trailers(buf, inst, &inner);
        emit_bind_this_assignment(buf, source, inst, &inst_local, &inner);
        emit_on_event_calls(buf, source, inst, &inst_local, &inner);
        return;
    }

    if snippet_children.is_empty() {
        let _ = write!(buf, "{inner}{ctor_lhs}");
        let call_start = buf.len() as u32;
        let _ = write!(buf, "new {local}({{ target: __svn_any(), props: {{");
        let mut first = true;
        for p in &inst.props {
            if !first {
                let _ = write!(buf, ", ");
            }
            first = false;
            write_prop_shape(buf, source, p);
        }
        if emit_implicit_children {
            if !first {
                let _ = write!(buf, ", ");
            }
            let _ = write!(buf, "children: () => __svn_snippet_return()");
        }
        let _ = write!(buf, "}} }})");
        push_component_call_token_map(buf, call_start, inst.node_start);
        buf.push_str(";\n");
        emit_component_bind_widen_trailers(buf, inst, &inner);
        emit_bind_this_assignment(buf, source, inst, &inst_local, &inner);
        emit_on_event_calls(buf, source, inst, &inst_local, &inner);
        return;
    }

    // Multi-line form with snippets-as-arrow-props.
    let _ = write!(buf, "{inner}{ctor_lhs}");
    let call_start = buf.len() as u32;
    let _ = writeln!(buf, "new {local}({{");
    let opts_inner = "    ".repeat(depth + 2);
    let props_inner = "    ".repeat(depth + 3);
    let _ = writeln!(buf, "{opts_inner}target: __svn_any(),");
    let _ = writeln!(buf, "{opts_inner}props: {{");
    for p in &inst.props {
        buf.push_str(&props_inner);
        write_prop_shape(buf, source, p);
        let _ = writeln!(buf, ",");
    }
    for s in snippet_children {
        buf.push_str(&props_inner);
        write_snippet_arrow_prop(buf, source, s, depth + 3, insts, action_counter);
        let _ = writeln!(buf, ",");
    }
    if emit_implicit_children {
        buf.push_str(&props_inner);
        let _ = writeln!(buf, "children: () => __svn_snippet_return(),");
    }
    let _ = writeln!(buf, "{opts_inner}}},");
    let _ = write!(buf, "{inner}}})");
    push_component_call_token_map(buf, call_start, inst.node_start);
    buf.push_str(";\n");
    emit_component_bind_widen_trailers(buf, inst, &inner);
    emit_bind_this_assignment(buf, source, inst, &inst_local, &inner);
    emit_on_event_calls(buf, source, inst, &inst_local, &inner);
    let _ = indent;
}

/// Emit the trailing `}` that closes a component-call block opened by
/// `emit_component_call`. Called by the template walker AFTER walking
/// the component's children, so user-side `{@const}` / `let:`-bound
/// names live inside the component-call's block scope — sibling
/// components each get a fresh scope.
fn emit_component_call_close(buf: &mut EmitBuffer, depth: usize) {
    let indent = "    ".repeat(depth);
    let _ = writeln!(buf, "{indent}}}");
}

/// Emit `void (() => { TARGET = __svn_any(null); });` after the
/// component's `new` expression for each simple-identifier
/// `bind:NAME={TARGET}` on this instantiation. TS flow analysis sees
/// the assignment inside the uncalled arrow and widens TARGET's
/// inferred type to `any` — models the Svelte runtime's async prop
/// writeback that TS can't see statically.
fn emit_component_bind_widen_trailers(
    buf: &mut EmitBuffer,
    inst: &svn_analyze::ComponentInstantiation,
    inner: &str,
) {
    for target in &inst.component_bind_widen_targets {
        // Wrap in ignore-region markers so any diagnostic firing
        // inside this purely-synthetic trailer is filtered out at
        // mapping time. Mirrors upstream svelte2tsx's
        // `/*Ωignore_startΩ*/ () => target = __sveltets_2_any(null);
        // /*Ωignore_endΩ*/` shape.
        let _ = writeln!(
            buf,
            "{inner}/*svn:ignore_start*/void (() => {{ {target} = __svn_any(null); }});/*svn:ignore_end*/"
        );
    }
}

/// Push a TokenMapEntry for a `new __svn_C_<hex>(...)` component call.
/// Source range is the 1-byte span at `node_start+1` — the first char
/// of the component name after `<`.
fn push_component_call_token_map(buf: &mut EmitBuffer, call_start: u32, node_start: u32) {
    let call_end = buf.len() as u32;
    let source_start = node_start.saturating_add(1);
    let source_end = source_start.saturating_add(1);
    buf.push_token_map(TokenMapEntry {
        overlay_byte_start: call_start,
        overlay_byte_end: call_end,
        source_byte_start: source_start,
        source_byte_end: source_end,
    });
}

/// Extract the NAME from a `PropShape`, if it has one. Used at emit
/// time to detect whether the user explicitly named a prop we'd
/// otherwise synthesize (e.g. `children`).
fn prop_shape_name(p: &svn_analyze::PropShape) -> Option<&str> {
    match p {
        svn_analyze::PropShape::Literal { name, .. }
        | svn_analyze::PropShape::Expression { name, .. }
        | svn_analyze::PropShape::Shorthand { name, .. }
        | svn_analyze::PropShape::BoolShorthand { name, .. }
        | svn_analyze::PropShape::GetSetBinding { name, .. } => Some(name),
        svn_analyze::PropShape::Spread { .. } => None,
    }
}

/// Emit one `$inst.$on("event", (handler))` line per `on:event`
/// directive on this component. Mirrors upstream svelte2tsx's shape.
/// Handler expression type-checks against the component's declared
/// `Events` record via `SvelteComponent<Props, Events, Slots>.$on`'s
/// signature.
fn emit_on_event_calls(
    buf: &mut EmitBuffer,
    source: &str,
    inst: &svn_analyze::ComponentInstantiation,
    inst_local: &str,
    inner: &str,
) {
    for ev in &inst.on_events {
        let expr = &source[ev.handler_range.start as usize..ev.handler_range.end as usize];
        let name = &ev.event_name;
        let _ = write!(buf, "{inner}{inst_local}.$on(\"{name}\", (");
        buf.append_with_source(expr, ev.handler_range);
        buf.push_str("));\n");
    }
}

/// Emit a `<EXPR> = $inst;` assignment for `bind:this={EXPR}` on a
/// component. EXPR is the verbatim user-source range — covers both
/// simple-identifier (`bind:this={refs}`) and member-expression
/// (`bind:this={refs.instance}`) forms.
fn emit_bind_this_assignment(
    buf: &mut EmitBuffer,
    source: &str,
    inst: &svn_analyze::ComponentInstantiation,
    inst_local: &str,
    inner: &str,
) {
    if let Some(range) = &inst.bind_this_target {
        let Some(expr) = source.get(range.start as usize..range.end as usize) else {
            return;
        };
        if expr.trim().is_empty() {
            return;
        }
        buf.push_str(inner);
        buf.append_with_source(expr, *range);
        buf.push_str(" = ");
        buf.push_str(inst_local);
        buf.push_str(";\n");
    }
}

/// Write a single property of a component-prop-check object literal,
/// dispatching on the analyze-side `PropShape` variant.
fn write_prop_shape(buf: &mut EmitBuffer, source: &str, p: &svn_analyze::PropShape) {
    let attr_range = p.attr_range();
    match p {
        svn_analyze::PropShape::Literal { name, value, .. } => {
            // CSS custom-property prop (`--foo-bar="#fff"`) — Svelte 5
            // applies these as CSS variables on the component's
            // wrapper element, not as Props. Mirrors upstream's
            // `__sveltets_2_cssProp(...)` treatment.
            if is_css_custom_prop_name(name) {
                buf.push_str("...__svn_css_prop({");
                write_quoted_prop_key_with_source(buf, name, attr_range);
                buf.push_str(": ");
                write_js_string_literal(buf, value);
                buf.push_str("})");
                return;
            }
            write_quoted_prop_key_with_source(buf, name, attr_range);
            buf.push_str(": ");
            write_js_string_literal(buf, value);
        }
        svn_analyze::PropShape::Expression {
            name, expr_range, ..
        } => {
            let expr = &source[expr_range.start as usize..expr_range.end as usize];
            if is_css_custom_prop_name(name) {
                buf.push_str("...__svn_css_prop({");
                write_quoted_prop_key_with_source(buf, name, attr_range);
                buf.push_str(": (");
                buf.append_with_source(expr, *expr_range);
                buf.push_str(")})");
                return;
            }
            write_quoted_prop_key_with_source(buf, name, attr_range);
            buf.push_str(": (");
            buf.append_with_source(expr, *expr_range);
            buf.push_str(")");
        }
        svn_analyze::PropShape::Shorthand { name, .. } => {
            // `{foo}` shorthand is only valid when the key is also a
            // valid JS identifier — otherwise expand to `"foo": foo`.
            if is_simple_js_identifier(name) {
                buf.append_with_source(name, attr_range);
            } else {
                write_quoted_prop_key_with_source(buf, name, attr_range);
                let _ = write!(buf, ": {name}");
            }
        }
        svn_analyze::PropShape::BoolShorthand { name, .. } => {
            write_quoted_prop_key_with_source(buf, name, attr_range);
            buf.push_str(": true");
        }
        svn_analyze::PropShape::Spread { expr_range, .. } => {
            let expr = &source[expr_range.start as usize..expr_range.end as usize];
            buf.push_str("...(");
            buf.append_with_source(expr, *expr_range);
            buf.push_str(")");
        }
        svn_analyze::PropShape::GetSetBinding {
            name,
            getter_range,
            setter_range,
            ..
        } => {
            let getter = &source[getter_range.start as usize..getter_range.end as usize];
            let setter = &source[setter_range.start as usize..setter_range.end as usize];
            write_quoted_prop_key_with_source(buf, name, attr_range);
            // Svelte 5 `bind:name={get, set}` — emit through the
            // `__svn_get_set_binding(get, set)` helper so TS infers
            // `T` from the getter's return, checks the setter's
            // parameter against `T`, and flows the return out to the
            // prop slot.
            buf.push_str(": __svn_get_set_binding(");
            buf.append_with_source(getter, *getter_range);
            buf.push_str(", ");
            buf.append_with_source(setter, *setter_range);
            buf.push_str(")");
        }
    }
}

/// Quoted-key emit for component prop literals — matches upstream
/// svelte2tsx's component-instantiation prop shape, so tsgo's TS2353
/// echoes the key as `'"name"'` rather than `'name'`. Pushes a
/// TokenMapEntry covering the synthesized `"name"` text in the overlay
/// pointing to the user's attribute span so prop-check diagnostics land
/// at the user's source position.
fn write_quoted_prop_key_with_source(
    buf: &mut EmitBuffer,
    name: &str,
    attr_range: svn_core::Range,
) {
    let mut quoted = String::with_capacity(name.len() + 2);
    write_js_string_literal_into_string(&mut quoted, name);
    buf.append_with_source(&quoted, attr_range);
}

/// Split `params_text` on top-level commas and, for each part that
/// doesn't already carry a top-level type annotation, append `: any`.
/// "Top-level" here means depth 0 in balanced `()`, `[]`, `{}`, `<>` —
/// so object-destructure patterns like `{ a, b }` are treated as one
/// unannotated part and get `{ a, b }: any` appended.
///
/// Used only by top-level `{#snippet}` blocks whose params have no
/// parent `Snippet<[...]>` contextual type to flow from.
pub(crate) fn annotate_snippet_params(params_text: &str) -> String {
    let bytes = params_text.as_bytes();
    let mut parts: Vec<(usize, usize)> = Vec::new();
    let mut depth = 0i32;
    let mut start = 0;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' | b'<' => depth += 1,
            b')' | b']' | b'}' | b'>' if depth > 0 => depth -= 1,
            b',' if depth == 0 => {
                parts.push((start, i));
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push((start, bytes.len()));

    let mut out = String::with_capacity(params_text.len() + 16);
    let mut first = true;
    for (s, e) in parts {
        if !first {
            out.push_str(", ");
        }
        first = false;
        let part = params_text[s..e].trim();
        if part.is_empty() {
            continue;
        }
        let part_bytes = part.as_bytes();
        let mut d = 0i32;
        let mut has_top_colon = false;
        for &b in part_bytes {
            match b {
                b'(' | b'[' | b'{' | b'<' => d += 1,
                b')' | b']' | b'}' | b'>' if d > 0 => d -= 1,
                b':' if d == 0 => {
                    has_top_colon = true;
                    break;
                }
                _ => {}
            }
        }
        out.push_str(part);
        if !has_top_colon {
            if let Some(eq) = find_top_level_eq(part) {
                let before = part[..eq].trim_end();
                let after = &part[eq..];
                out.truncate(out.len() - part.len());
                out.push_str(before);
                out.push_str(": any ");
                out.push_str(after);
            } else {
                out.push_str(": any");
            }
        }
    }
    out
}

/// First top-level `=` in `s`, or `None`. Skips `==` / `===` / `=>`.
fn find_top_level_eq(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut d = 0i32;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'(' | b'[' | b'{' | b'<' => d += 1,
            b')' | b']' | b'}' | b'>' if d > 0 => d -= 1,
            b'=' if d == 0 => {
                let next = bytes.get(i + 1).copied();
                if next == Some(b'=') || next == Some(b'>') {
                    continue;
                }
                return Some(i);
            }
            _ => {}
        }
    }
    None
}

/// Write a `{#snippet name(params)}...{/snippet}` block as an
/// arrow-callback property value on the parent component's call-site
/// props object:
///
/// ```ts
///     name: (params) => {
///         <snippet body>
///         return __svn_snippet_return();
///     }
/// ```
///
/// The parameter text is spliced verbatim from the source — that
/// preserves user-provided type annotations, destructure patterns,
/// and default values. TypeScript's contextual typing reads the parent
/// component's declared `Snippet<[...]>` parameter shape through the
/// call signature's props slot and flows each tuple element into the
/// matching arrow parameter, so destructured snippet params pick up
/// real types instead of implicit-any.
fn write_snippet_arrow_prop(
    buf: &mut EmitBuffer,
    source: &str,
    s: &SnippetBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let body_indent = "    ".repeat(depth + 1);
    let params_text = source
        .get(s.parameters_range.start as usize..s.parameters_range.end as usize)
        .unwrap_or("")
        .trim();
    write_object_key(buf, &s.name);
    if params_text.is_empty() {
        let _ = writeln!(buf, ": () => {{");
        emit_template_body(buf, source, &s.body, depth + 1, insts, action_counter);
        let _ = writeln!(buf, "{body_indent}return __svn_snippet_return();");
        let _ = write!(buf, "{indent}}}");
        return;
    }
    let _ = writeln!(buf, ": ({params_text}) => {{");
    emit_template_body(buf, source, &s.body, depth + 1, insts, action_counter);
    for ident in all_identifiers(params_text) {
        let _ = writeln!(buf, "{body_indent}void {ident};");
    }
    let _ = writeln!(buf, "{body_indent}return __svn_snippet_return();");
    let _ = write!(buf, "{indent}}}");
}

/// Write an object-literal key. Plain JS identifiers are emitted bare;
/// anything with a hyphen, a non-ident character, or a JS reserved
/// word lookalike is double-quoted (always safe).
fn write_object_key(buf: &mut EmitBuffer, name: &str) {
    if is_simple_js_identifier(name) {
        buf.push_str(name);
    } else {
        write_js_string_literal(buf, name);
    }
}

/// Write `value` as a JS double-quoted string literal, escaping the
/// usual unsafe characters. Pure ASCII assumption — Svelte attribute
/// values are decoded earlier in the pipeline.
fn write_js_string_literal(buf: &mut EmitBuffer, value: &str) {
    buf.push('"');
    for c in value.chars() {
        match c {
            '"' => buf.push_str("\\\""),
            '\\' => buf.push_str("\\\\"),
            '\n' => buf.push_str("\\n"),
            '\r' => buf.push_str("\\r"),
            '\t' => buf.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(buf, "\\u{:04x}", c as u32);
            }
            c => buf.push(c),
        }
    }
    buf.push('"');
}

/// String-builder twin of [`write_js_string_literal`] used by
/// `write_quoted_prop_key_with_source`'s pre-format step. Escapes
/// the value into a freshly-allocated `String` so the caller can
/// hand it as a single chunk to `EmitBuffer::append_with_source`.
fn write_js_string_literal_into_string(out: &mut String, value: &str) {
    out.push('"');
    for c in value.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}
