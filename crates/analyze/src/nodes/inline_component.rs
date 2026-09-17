//! `<Component>` analyze pass — mirrors upstream
//! `htmlxtojsx_v2/nodes/InlineComponent.ts`.

use smol_str::SmolStr;
use svn_core::Range;
use svn_parser::{AttrValuePart, Attribute, Component, Node};

use crate::nodes::attribute::{WalkCtx, walk_attributes};
use crate::nodes::destructure::simple_identifier_in;
use crate::walker::{
    AnalyzeVisitor, BindDirective, BubbledComponentEvent, CommentThread, ComponentInstantiation,
    OnEventDirective, PropShape, TemplateSummary, ThreadedComment,
};

pub(crate) fn visit(v: &mut AnalyzeVisitor<'_>, c: &Component) {
    let ctx = WalkCtx { source: v.source };
    // `use:` on a component is nonsensical at the Svelte level
    // (actions attach to DOM elements, not to component instances),
    // but we pass it along — emit's shim-side
    // `__svn_map_element_tag(tag: string)` overload resolves
    // unknown tags to `HTMLElement` so the pattern doesn't break
    // the program.
    walk_attributes(&c.attributes, &mut v.summary, &mut v.counters, &ctx, None);
    collect_component_instantiation(c, v.source, &mut v.summary);
    crate::nodes::let_directive::enter_component_like(
        v,
        c.range.start,
        &c.attributes,
        &c.children,
        Some(c.name.clone()),
    );
}

/// Inspect a `<Component ...>` site and, if it's a shape we know how to
/// generate a satisfies-check for, push a `ComponentInstantiation` to
/// the summary.
///
/// Handles both simple component names (`<MyButton />`) and dotted
/// forms (`<ui.MyButton />`, `<UI.TextInput>`). The dotted form is
/// passed verbatim through to the emit's `__svn_ensure_component(...)`
/// call — `UI.TextInput` evaluates to the member-referenced component
/// value, which the ensure_component overloads resolve the same way
/// as a simple-identifier reference. Emit also voids the root
/// identifier via template_refs, so the barrel import isn't flagged
/// unused.
///
/// Each plain attribute (including boolean shorthand and `{shorthand}`)
/// contributes a `PropShape` to the literal. Directive attributes
/// (`bind:`, `on:`, `use:`, `class:`, `style:`, `transition:`, etc.)
/// and spreads (`{...obj}`) are silently DROPPED — they provide
/// runtime values that we can't model statically. Their absence from
/// the literal is harmless because emit wraps the satisfies target
/// in `Partial<>`, so missing-required-prop never fires; only the
/// explicit props the user wrote get checked for excess.
///
/// One disqualifier remains: a plain attribute with a multi-part value
/// (`class="a {b} c"`-style interpolation in a quoted attr). The value
/// isn't representable as a single TS expression without re-emitting
/// the template's interpolation pipeline; the whole instantiation is
/// skipped so the satisfies object stays correct on the rest.
pub(crate) fn collect_component_instantiation(
    c: &svn_parser::Component,
    source: &str,
    summary: &mut TemplateSummary,
) {
    // The tag name starts right after `<`.
    let name_start = c.range.start + 1;
    let name_end = name_start + c.name.len() as u32;
    collect_instantiation_inner(
        InstantiationRoot {
            text: c.name.clone(),
            range: Range::new(name_start, name_end),
            ctor_anchor: Range::new(name_end.saturating_sub(1), name_end),
        },
        &c.attributes,
        &c.children,
        c.range.start,
        source,
        summary,
    );
}

/// What an instantiation constructs from: the root text emit passes to
/// `__svn_ensure_component(...)`, and the source positions its
/// diagnostics map to (see [`ComponentInstantiation::ctor_anchor`]).
pub(crate) struct InstantiationRoot {
    pub(crate) text: SmolStr,
    pub(crate) range: Range,
    pub(crate) ctor_anchor: Range,
}

/// Body of [`collect_component_instantiation`] generalised to accept
/// the parts directly. Reused by `<svelte:component this={X}>` and
/// `<svelte:self>` paths which don't have a `Component` AST node but
/// still want excess-prop / on-event / bind:this checking through
/// the same machinery. The `component_root` string is what emit
/// passes to `__svn_ensure_component(...)`; for synthetic kinds
/// (svelte:self → `__svn_self_default`, svelte:component →
/// `(EXPR)`), emit recognises the synthetic form and routes
/// accordingly.
pub(crate) fn collect_instantiation_inner(
    root: InstantiationRoot,
    attributes: &[Attribute],
    children: &svn_parser::Fragment,
    range_start: u32,
    source: &str,
    summary: &mut TemplateSummary,
) {
    let InstantiationRoot {
        text: component_root,
        range: root_range,
        ctor_anchor,
    } = root;
    let mut props: Vec<PropShape> = Vec::with_capacity(attributes.len());
    let mut on_events: Vec<OnEventDirective> = Vec::new();
    let mut bind_this_target: Option<Range> = None;
    let mut bind_this_setter: Option<Range> = None;
    let mut component_bind_widen_targets: Vec<SmolStr> = Vec::new();
    let mut bind_directives: Vec<BindDirective> = Vec::new();
    // Implicit `children`: upstream `SnippetBlock.ts`
    // `handleImplicitChildren` fakes a `children` prop when any child
    // other than a snippet, a comment, a `<slot>`, blank text, or an
    // element / component / `<svelte:fragment>` placed in a named slot
    // (`slot="x"`, `x` ≠ `default`) sits between the tags.
    let has_implicit_children = children.nodes.iter().any(|n| match n {
        Node::SnippetBlock(_) | Node::Comment(_) => false,
        Node::Text(t) => !t.range.slice(source).trim().is_empty(),
        Node::Element(e) => e.name.as_str() != "slot" && !in_named_slot(&e.attributes, source),
        Node::Component(c) => !in_named_slot(&c.attributes, source),
        Node::SvelteElement(e) => !in_named_slot(&e.attributes, source),
        _ => true,
    });
    let implicit_children_anchor = if has_implicit_children {
        let name_move = (component_root.as_str() != "__svn_self_default").then_some(root_range);
        children.nodes.first().map(|first| {
            implicit_children_anchor(
                StartTag {
                    start: range_start,
                    end: first.range().start,
                    name_move,
                },
                attributes,
                source,
            )
        })
    } else {
        None
    };
    let mut prop_comments: Vec<(Range, CommentThread)> = Vec::new();
    for (index, attr) in attributes.iter().enumerate() {
        let comments = comment_thread(attributes, index, source);
        let props_before = props.len();
        let events_before = on_events.len();
        collect_attribute(
            attr,
            source,
            &component_root,
            summary,
            AttributeSinks {
                props: &mut props,
                on_events: &mut on_events,
                bind_this_target: &mut bind_this_target,
                bind_this_setter: &mut bind_this_setter,
                component_bind_widen_targets: &mut component_bind_widen_targets,
                bind_directives: &mut bind_directives,
            },
        );
        if comments.is_empty() {
            continue;
        }
        if props.len() > props_before
            && let Some(p) = props.last()
        {
            prop_comments.push((p.attr_range(), comments));
        } else if on_events.len() > events_before
            && let Some(ev) = on_events.last_mut()
        {
            ev.comments = comments;
        }
    }
    summary
        .component_instantiations
        .push(ComponentInstantiation {
            component_root,
            root_range,
            ctor_anchor,
            props,
            has_implicit_children,
            on_events,
            bind_this_target,
            bind_this_setter,
            component_bind_widen_targets,
            bind_directives,
            prop_comments,
            implicit_children_anchor,
            node_start: range_start,
        });
}

/// The parts of a component start tag svelte2tsx's rewrite of it
/// depends on.
struct StartTag {
    /// Byte offset of the `<`.
    start: u32,
    /// Where the start tag's rewrite ends: the first child's start.
    end: u32,
    /// The source range moved in as the constructed component value
    /// (the tag name, or `<svelte:component>`'s `this` expression);
    /// `None` for `<svelte:self>`.
    name_move: Option<Range>,
}

/// Source offset the synthesized implicit `children` prop maps to.
///
/// svelte2tsx rewrites a component start tag by moving the source
/// ranges it keeps (the name, attribute names and values, directive
/// expressions, comments) behind the tag and blanking what lies
/// between them (`htmlxtojsx_v2/utils/node-utils.ts` `transform`). The
/// `children` prop is plain inserted text with no mapping of its own,
/// so a diagnostic on it resolves to the source of whatever mapped
/// text precedes it in the output:
///
/// - with no whitespace right after the tag name, that is the name's
///   last character;
/// - otherwise the whitespace character after the name is kept as a
///   range, and `children` is written over the character following it
///   — unless another range starts there or it is the tag's last
///   character;
/// - failing that, the text before `children` is the last blanked gap
///   the rewrite moved behind the tag, which maps to the gap's first
///   character (or the name's last character when there is none).
fn implicit_children_anchor(tag: StartTag, attributes: &[Attribute], source: &str) -> Range {
    let bytes = source.as_bytes();
    let tag_name_len = source
        .get(tag.start as usize + 1..)
        .map(|rest| {
            rest.find(|ch: char| ch.is_whitespace() || ch == '/' || ch == '>')
                .unwrap_or(rest.len())
        })
        .unwrap_or(0) as u32;
    let tag_name_end = tag.start + 1 + tag_name_len;
    let name_last = match tag.name_move {
        Some(r) if r.end > r.start => r.end - 1,
        _ => tag_name_end.saturating_sub(1),
    };
    let one = |at: u32| Range::new(at, at + 1);
    if !bytes
        .get(tag_name_end as usize)
        .is_some_and(|b| b.is_ascii_whitespace())
    {
        return one(name_last);
    }
    let mut ranges: Vec<(u32, u32)> = Vec::with_capacity(attributes.len() * 2 + 2);
    if let Some(r) = tag.name_move {
        ranges.push((r.start, r.end));
    }
    ranges.push((tag_name_end, tag_name_end + 1));
    for (index, attr) in attributes.iter().enumerate() {
        let thread = comment_thread(attributes, index, source);
        let shorthand = matches!(attr, Attribute::Shorthand(_));
        if !shorthand {
            ranges.extend(thread.leading.iter().map(|c| (c.range.start, c.range.end)));
        }
        kept_attribute_ranges(attr, source, &mut ranges);
        ranges.extend(thread.trailing.iter().map(|c| (c.range.start, c.range.end)));
    }
    let end = tag.end;
    let starts_at = |pos: u32| ranges.iter().any(|&(s, _)| s == pos);
    let mut moved: Vec<(u32, u32)> = ranges
        .iter()
        .filter(|(s, e)| s != e)
        .map(|&(s, e)| {
            if e + 1 < end && !starts_at(e) {
                (s, e + 1)
            } else {
                (s, e)
            }
        })
        .collect();
    let blank_end = tag_name_end + 1;
    if blank_end + 1 < end && !starts_at(blank_end) {
        return one(blank_end);
    }
    moved.sort_unstable();
    let mut last_gap = None;
    let mut remove_start = tag.start;
    for &(s, e) in &moved {
        if remove_start < s && remove_start > tag_name_end && s < end {
            last_gap = Some(remove_start);
        }
        remove_start = e;
    }
    if remove_start < end {
        remove_start += 1;
        if remove_start > tag_name_end && remove_start + 1 < end {
            last_gap = Some(remove_start);
        }
    }
    one(last_gap.unwrap_or(name_last))
}

/// The source ranges svelte2tsx keeps from one component attribute.
fn kept_attribute_ranges(attr: &Attribute, source: &str, out: &mut Vec<(u32, u32)>) {
    let trimmed = |r: Range| {
        let text = r.slice(source);
        let start = r.start + (text.len() - text.trim_start().len()) as u32;
        (start, start + text.trim().len() as u32)
    };
    let name_range = |start: u32, name: &str| (start, start + name.len() as u32);
    match attr {
        Attribute::Comment(_) => {}
        Attribute::Plain(p) => {
            // `slot="x"` under a component names the slot the element
            // fills; its value is still a kept range.
            let is_slot_name = p.name.as_str() == "slot";
            if !is_slot_name {
                out.push(name_range(p.range.start, p.name.as_str()));
            }
            let Some(value) = &p.value else {
                return;
            };
            match value.parts.as_slice() {
                [] => {}
                [svn_parser::AttrValuePart::Text { range }] => {
                    if range.start == range.end {
                        out.push((range.start.saturating_sub(1), range.end + 1));
                    } else {
                        out.push((range.start, range.end));
                    }
                }
                [
                    svn_parser::AttrValuePart::Expression {
                        expression_range, ..
                    },
                ] if !is_slot_name => out.push(trimmed(*expression_range)),
                [first, .., last] if !is_slot_name => {
                    let start = match first {
                        svn_parser::AttrValuePart::Text { range } => range.start,
                        svn_parser::AttrValuePart::Expression { range, .. } => range.start,
                    };
                    let end = match last {
                        svn_parser::AttrValuePart::Text { range } => range.end,
                        svn_parser::AttrValuePart::Expression { range, .. } => range.end,
                    };
                    out.push((start, end));
                }
                _ => {}
            }
        }
        Attribute::Expression(e) => {
            out.push(name_range(e.range.start, e.name.as_str()));
            out.push(trimmed(e.expression_range));
        }
        Attribute::Shorthand(s) => {
            out.push(trimmed(Range::new(s.range.start + 1, s.range.end - 1)))
        }
        Attribute::Spread(s) => out.push((s.range.start + 1, s.range.end.saturating_sub(1))),
        Attribute::Directive(d) => {
            let prefix = d.kind.prefix_len_with_colon();
            let name = name_range(d.range.start + prefix, d.name.as_str());
            match (&d.kind, &d.value) {
                (svn_parser::DirectiveKind::Bind, None) => out.push(name),
                (
                    svn_parser::DirectiveKind::Bind,
                    Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }),
                ) => {
                    if d.name.as_str() != "this" {
                        let eq = source
                            .get(..expression_range.start as usize)
                            .and_then(|s| s.rfind('='))
                            .map_or(name.1, |i| i as u32);
                        out.push((name.0, eq));
                    }
                    out.push(trimmed(*expression_range));
                }
                (
                    svn_parser::DirectiveKind::Bind,
                    Some(svn_parser::DirectiveValue::BindPair {
                        getter_range,
                        setter_range,
                        ..
                    }),
                ) => {
                    if d.name.as_str() == "this" {
                        out.push(trimmed(*setter_range));
                    } else {
                        out.push(name);
                        out.push(trimmed(*getter_range));
                        out.push(trimmed(*setter_range));
                    }
                }
                (svn_parser::DirectiveKind::On | svn_parser::DirectiveKind::Let, value) => {
                    out.push(name);
                    if let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = value
                    {
                        out.push(trimmed(*expression_range));
                    }
                }
                (svn_parser::DirectiveKind::Class, value) => match value {
                    Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) => out.push(trimmed(*expression_range)),
                    _ => out.push(name),
                },
                (svn_parser::DirectiveKind::Style, value) => match value {
                    None => out.push((name.0, d.range.end)),
                    Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) => out.push((expression_range.start, expression_range.end)),
                    Some(svn_parser::DirectiveValue::Quoted(v)) => {
                        let bounds = v.parts.first().zip(v.parts.last()).map(|(f, l)| {
                            let start = match f {
                                svn_parser::AttrValuePart::Text { range } => range.start,
                                svn_parser::AttrValuePart::Expression { range, .. } => range.start,
                            };
                            let end = match l {
                                svn_parser::AttrValuePart::Text { range } => range.end,
                                svn_parser::AttrValuePart::Expression { range, .. } => range.end,
                            };
                            (start, end)
                        });
                        out.extend(bounds);
                    }
                    Some(svn_parser::DirectiveValue::BindPair { .. }) => {}
                },
                // `use:` / transitions / `animate:` have no meaning on a
                // component and keep nothing.
                _ => {}
            }
        }
    }
}

/// Where [`collect_attribute`] records what one attribute contributes.
struct AttributeSinks<'a> {
    props: &'a mut Vec<PropShape>,
    on_events: &'a mut Vec<OnEventDirective>,
    bind_this_target: &'a mut Option<Range>,
    bind_this_setter: &'a mut Option<Range>,
    component_bind_widen_targets: &'a mut Vec<SmolStr>,
    bind_directives: &'a mut Vec<BindDirective>,
}

/// The in-tag comments belonging to `attributes[index]`: the comments
/// written directly before it (only whitespace between each of them
/// and the next), and — when it is the tag's last attribute — the
/// comments written after it, provided nothing but whitespace and an
/// optional `/` follows them up to the tag's `>`.
pub fn comment_thread(attributes: &[Attribute], index: usize, source: &str) -> CommentThread {
    let mut thread = CommentThread::default();
    let Some(attr) = attributes.get(index) else {
        return thread;
    };
    if matches!(attr, Attribute::Comment(_)) {
        return thread;
    }
    let blank = |from: u32, to: u32| {
        source
            .get(from as usize..to as usize)
            .is_some_and(|s| s.trim().is_empty())
    };
    let threaded = |range: Range| ThreadedComment {
        range,
        newline: starts_line(source, range.start),
    };
    let range = attr.range();
    let mut search_end = range.start;
    for prev in attributes[..index].iter().rev() {
        let Attribute::Comment(c) = prev else {
            break;
        };
        if !blank(c.range.end, search_end) {
            break;
        }
        thread.leading.insert(0, threaded(c.range));
        search_end = c.range.start;
    }
    let rest = &attributes[index + 1..];
    if !rest.iter().all(|a| matches!(a, Attribute::Comment(_))) {
        return thread;
    }
    let Some(tag_end) = source
        .get(range.end as usize..)
        .and_then(|s| s.find('>'))
        .map(|i| range.end + i as u32)
    else {
        return thread;
    };
    let mut trailing = Vec::new();
    let mut search_start = range.end;
    for next in rest {
        let Attribute::Comment(c) = next else {
            break;
        };
        if c.range.end > tag_end || !blank(search_start, c.range.start) {
            break;
        }
        trailing.push(threaded(c.range));
        search_start = c.range.end;
    }
    let tail = source
        .get(search_start as usize..tag_end as usize)
        .unwrap_or("x");
    if !trailing.is_empty() && tail.trim().trim_start_matches('/').trim().is_empty() {
        thread.trailing = trailing;
    }
    thread
}

/// Whether only spaces and tabs sit between the previous line break
/// and `pos`, looking back at most 100 bytes.
fn starts_line(source: &str, pos: u32) -> bool {
    let start = (pos as usize).saturating_sub(100);
    let Some(before) = source.get(start..pos as usize) else {
        return false;
    };
    let trimmed = before.trim_end_matches([' ', '\t']);
    trimmed.ends_with('\n')
}

/// Record what one attribute of a component start tag contributes to
/// the instantiation.
fn collect_attribute(
    attr: &Attribute,
    source: &str,
    component_root: &SmolStr,
    summary: &mut TemplateSummary,
    sinks: AttributeSinks<'_>,
) {
    let AttributeSinks {
        props,
        on_events,
        bind_this_target,
        bind_this_setter,
        component_bind_widen_targets,
        bind_directives,
    } = sinks;
    {
        match attr {
            Attribute::Plain(p) => {
                // SVELTE-4-COMPAT: `slot="x"` on a component is a
                // POSITIONAL marker for the Svelte compiler (places
                // the child into a named slot of its parent), not a
                // prop of the child itself. Emitting it as a prop
                // fires TS2353 on every Svelte-5 child that doesn't
                // declare a `slot` prop. Skip entirely — the
                // Svelte-4 widen on the ENCLOSING parent already
                // handles the case where `slot` *is* explicitly
                // passed as a prop name.
                if p.name.as_str() == "slot" {
                    return;
                }
                let Some(v) = &p.value else {
                    props.push(PropShape::BoolShorthand {
                        name: p.name.clone(),
                        attr_range: p.range,
                    });
                    return;
                };
                // Single literal text part (no interpolations) — keep it.
                if v.parts.len() == 1 {
                    if let AttrValuePart::Text { range } = &v.parts[0] {
                        props.push(PropShape::Literal {
                            name: p.name.clone(),
                            value: range.slice(source).to_string(),
                            attr_range: p.range,
                        });
                        return;
                    }
                }
                // Multi-part interpolated attribute value
                // (`class="a {b} c"`) — emit as a TS template
                // literal `\`a ${b} c\`` so the embedded
                // expressions get type-checked AND the prop's
                // value carries a real string type. Mirrors upstream
                // svelte2tsx's `Attribute.ts:233`.
                props.push(PropShape::TemplateLiteral {
                    name: p.name.clone(),
                    parts: v.parts.clone(),
                    attr_range: p.range,
                });
            }
            Attribute::Expression(e) => {
                props.push(PropShape::Expression {
                    name: e.name.clone(),
                    expr_range: e.expression_range,
                    attr_range: e.range,
                });
            }
            Attribute::Shorthand(s) => {
                // R-Conv #19 (D-ii fix #3): anchor 2353 / 2322 on the
                // NAME (`o` of `{only_bind}`), not the opening brace.
                // Upstream LS's reverse-map for `<Foo {only_bind} />`
                // points at the prop name's first byte — matches
                // `bindings` fixture line 28 col 8 vs ours pre-fix
                // col 7.
                let inner = source
                    .get(s.range.start as usize + 1..s.range.end as usize)
                    .unwrap_or("");
                let leading_ws = (inner.len() - inner.trim_start().len()) as u32;
                let name_start = s.range.start + 1 + leading_ws;
                let name_end = name_start + s.name.len() as u32;
                props.push(PropShape::Shorthand {
                    name: s.name.clone(),
                    attr_range: svn_core::Range::new(name_start, name_end),
                });
            }
            Attribute::Comment(_) => {}
            Attribute::Directive(d) => {
                // `on:event={handler}` on a component emits as
                // `$inst.$on("event", handler)` after construction
                // (mirrors upstream svelte2tsx). Handler's type
                // flows through `SvelteComponent<P, E, S>.$on`
                // against the declared Events type.
                if d.kind == svn_parser::DirectiveKind::On {
                    // `on:` prefix is 3 bytes; the name follows
                    // immediately (modifiers come after the name with
                    // `|` separators, stored in `d.modifiers`, NOT
                    // in `d.name`).
                    let name_start = d.range.start + 3;
                    let name_end = name_start + d.name.len() as u32;
                    let name_range = Range::new(name_start, name_end);
                    if let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                    {
                        on_events.push(OnEventDirective {
                            event_name: d.name.clone(),
                            name_range,
                            handler_range: *expression_range,
                            comments: CommentThread::default(),
                        });
                    } else {
                        // `on:event` with no value — bare re-dispatch
                        // (event bubbling from sub-component).
                        //
                        // Reviewer follow-up #1: push an
                        // `OnEventDirective` with an empty range so
                        // emit produces `$inst.$on("event", () => {})`
                        // — type-checks the bubbled-event name against
                        // the child's declared Events surface. Pre-fix
                        // we only set `has_bubbled_component_event`
                        // and skipped the $on call entirely, so a
                        // bubbled event with a NAME the child doesn't
                        // declare passed silently. Mirrors upstream
                        // svelte2tsx's `EventHandler.ts:147` shape.
                        //
                        // The `has_bubbled_component_event` flag stays
                        // — it drives the OUTER component's
                        // default-export Props-widen (separate
                        // upstream behavior: components that
                        // re-dispatch sub-component events get their
                        // Props widened to `Record<string, any>` per
                        // upstream's `with_any_event` /
                        // `isomorphic_component` inference fallback).
                        on_events.push(OnEventDirective {
                            event_name: d.name.clone(),
                            name_range,
                            handler_range: Range::new(d.range.start, d.range.start),
                            comments: CommentThread::default(),
                        });
                        // Round-7 follow-up #7: upstream's
                        // `event-handler.ts:12-15` skips
                        // `handleEventHandlerBubble` when the parent
                        // is `<svelte:self>` — bubbling self's own
                        // events into the wrapper's `$$Events` is a
                        // no-op (the parent component IS the child)
                        // and would wrongly disqualify the runes
                        // fn_component shape and trigger the
                        // Svelte-4 props-widen path. The `$inst.$on`
                        // call still fires above so the bubbled name
                        // type-checks against self's events surface;
                        // we just don't register a bubble for it.
                        if component_root.as_str() == "__svn_self_default" {
                            return;
                        }
                        summary.has_bubbled_component_event = true;
                        // Reviewer follow-up #2: also record the
                        // (event_name, component_root) pair so the
                        // wrapper's own `$$Events` surface carries the
                        // bubbled name. Emit projects via
                        // `__SvnComponentEvents<typeof <root>>["NAME"]`
                        // and intersects with `events_alias_body`.
                        summary
                            .bubbled_component_events
                            .push(BubbledComponentEvent {
                                event_name: d.name.clone(),
                                component_root: component_root.clone(),
                                position: d.range.start,
                            });
                    }
                    return;
                }
                // `bind:NAME={x}` on a component (other than
                // `bind:this`) is type-equivalent to passing `x` as
                // the `NAME` prop. Emit as a regular expression prop
                // so the child's `Props.NAME` declared type catches
                // mismatches (`<Child bind:value={x: string}>` when
                // `value` is declared `number` fires TS2322).
                //
                // `bind:this={x}` records the identifier so emit can
                // assign the component instance to it — `x =
                // $$_inst;` — after construction. TS checks `x`'s
                // declared type accepts the instance; mismatches
                // fire TS2322 ("Type 'MyComp' is not assignable to
                // type 'OtherComp'"). The existing definite-assign
                // `!` rewrite still fires via `walk_attributes`
                // (that runs first, on every attribute of every
                // node); this captures the instance variable name
                // in addition so the component-local emit can
                // reference it.
                //
                // The `bind:NAME={getter, setter}` get/set form
                // (`BindPair`) is handled by the dedicated branch
                // below — it never reaches this `bind:this` arm.
                if d.kind == svn_parser::DirectiveKind::Bind && d.name.as_str() == "this" {
                    if let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                    {
                        // Record the full expression range regardless
                        // of shape (simple identifier OR member
                        // expression). Emit renders source verbatim.
                        // The sibling `bind_this_targets` collection
                        // via `walk_directive` still filters for
                        // simple-identifier names — that's for the
                        // declaration-site `!` rewrite which only
                        // applies to simple `let` declarations.
                        *bind_this_target = Some(*expression_range);
                    }
                    // `bind:this={get, set}`: upstream calls the setter
                    // with the instance (`Binding.ts`).
                    if let Some(svn_parser::DirectiveValue::BindPair { setter_range, .. }) =
                        &d.value
                    {
                        *bind_this_setter = Some(*setter_range);
                    }
                    return;
                }
                if d.kind == svn_parser::DirectiveKind::Bind
                    && let Some(svn_parser::DirectiveValue::Expression {
                        expression_range, ..
                    }) = &d.value
                {
                    let target = d.name.clone();
                    // Dedup: if the user already wrote the same name
                    // as a plain attribute (`<Child value={x}
                    // bind:value={x} />` — redundant but seen), drop
                    // the prior entry so the bind: expression is the
                    // final authority.
                    props.retain(|p| p.name() != Some(&target));
                    // R-Conv #1: anchor diagnostics on the property
                    // NAME (`prop` in `bind:prop={…}`), not the
                    // whole `bind:prop={…}` slice. Upstream's LS
                    // reverse-mapping for component-bind sites
                    // points at the name (`$store-bind` fixture's
                    // expected col 16 = start of `prop`, not col 11
                    // = start of `bind:`). Using `d.range` here
                    // anchored 5 chars too early.
                    let name_start = d.range.start + d.kind.prefix_len_with_colon();
                    let name_end = name_start + target.len() as u32;
                    let name_range = svn_core::Range::new(name_start, name_end);
                    props.push(PropShape::Expression {
                        name: target,
                        expr_range: *expression_range,
                        attr_range: name_range,
                    });
                    // Widen target if the expression is a simple
                    // identifier — emit's post-`new` trailer will write
                    // `() => <ident> = __svn_any(null);` so TS flow
                    // analysis widens the target's type to `any`. Only
                    // simple identifiers are safe; member expressions
                    // (`bind:prop={x.y}`) and destructures aren't
                    // assignable in a one-liner arrow without matching
                    // the exact declaration shape.
                    if let Some(ident) = simple_identifier_in(source, *expression_range) {
                        component_bind_widen_targets.push(ident);
                    }
                    // R-Conv #19 (D-ii fix #4): record the prop NAME +
                    // `bind:NAME` source range so emit can write
                    // `__svn_inst_N.$$bindings = 'NAME';` post-instance
                    // for the literal-Bindings union check.
                    bind_directives.push(BindDirective {
                        name: d.name.clone(),
                        range: d.range,
                    });
                    return;
                }
                // Bare shorthand `bind:NAME` desugars to
                // `bind:NAME={NAME}` — emit as a Shorthand prop so
                // phase 5's satisfies sees the required field
                // present. Without this branch a
                // `<CustomFieldModal bind:items />` consumer fails
                // the satisfies with "Property 'items' missing"
                // despite the user correctly binding. Mirrors the
                // explicit-expression arm above.
                if d.kind == svn_parser::DirectiveKind::Bind && d.value.is_none() {
                    let target = d.name.clone();
                    props.retain(|p| p.name() != Some(&target));
                    // Bare `bind:NAME` is `bind:NAME={NAME}` — same
                    // widening trailer as the explicit form.
                    component_bind_widen_targets.push(target.clone());
                    // R-Conv #19 (D-ii fix #3): anchor diagnostics on
                    // the NAME, not the `bind:` prefix. Mirrors the
                    // explicit-expression arm at line 2324 — upstream
                    // LS's reverse-map points at the prop name (e.g.
                    // `bindings` fixture line 27 col 12 = `o` of
                    // `only_bind`, not col 7 = `b` of `bind:`).
                    let name_start = d.range.start + d.kind.prefix_len_with_colon();
                    let name_end = name_start + target.len() as u32;
                    let name_range = svn_core::Range::new(name_start, name_end);
                    props.push(PropShape::Shorthand {
                        name: target.clone(),
                        attr_range: name_range,
                    });
                    bind_directives.push(BindDirective {
                        name: target,
                        range: d.range,
                    });
                    return;
                }
                // Svelte 5 `bind:NAME={getter, setter}` get/set form
                // (DirectiveValue::BindPair). Upstream svelte2tsx uses
                // a `__sveltets_2_get_set_binding` helper to model
                // it; we don't yet. Without this branch v0.3's
                // satisfies trailer catches `name` / `definition`
                // etc. as "missing required props" on consumers that
                // correctly use the get/set form — false positives
                // on Svelte 5 bind: idiom.
                //
                // Interim: push the name as Shorthand so satisfies
                // Emit site lowers this to `name: __svn_get_set_binding(
                // <getter>, <setter>)` — mirrors upstream's
                // `__sveltets_2_get_set_binding` helper so TS infers `T`
                // from the getter's return AND checks the setter's
                // parameter against the same `T`. See
                // `design/get_set_binding/` for the fixture-locked shape.
                if d.kind == svn_parser::DirectiveKind::Bind
                    && let Some(svn_parser::DirectiveValue::BindPair {
                        getter_range,
                        setter_range,
                        ..
                    }) = &d.value
                {
                    let target = d.name.clone();
                    props.retain(|p| p.name() != Some(&target));
                    props.push(PropShape::GetSetBinding {
                        name: target.clone(),
                        getter_range: *getter_range,
                        setter_range: *setter_range,
                        attr_range: d.range,
                    });
                    bind_directives.push(BindDirective {
                        name: target,
                        range: d.range,
                    });
                }
                // Other directives (`use:`, `class:`, `style:`,
                // transitions, animations) are runtime behaviors
                // with no type-level surface on components. Drop.
            }
            // Spread — silently dropped. The Partial<> wrap in emit
            // means we don't need to model the props it would
            // contribute; we only check what the user wrote explicitly.
            Attribute::Spread(s) => {
                // `<Comp {...rest}>` contributes whatever `rest` holds
                // at runtime. Emit as a spread in the props literal so
                // TS structurally type-checks `rest`'s inferred shape
                // against the declared Props — missing-required-prop
                // errors surface on the spread expression itself
                // (useful user-facing signal). A spread CAN fill
                // required-but-not-named-elsewhere props, which is
                // also why phase 5's `satisfies` trailer is
                // tolerant-by-spread without false-positive.
                props.push(PropShape::Spread {
                    expr_range: s.expression_range,
                    attr_range: s.range,
                });
            }
        }
    }
}

/// `slot="x"` with `x` other than `default` on a component child — the
/// child fills a named slot of the component, so it is not part of its
/// `children`.
fn in_named_slot(attributes: &[Attribute], source: &str) -> bool {
    // `a.value[0]?.data !== 'default'` (`SnippetBlock.ts`): only a text
    // value reading exactly `default` keeps the child in the default
    // slot; a bare, empty or `{…}` value does not.
    attributes.iter().any(|a| match a {
        Attribute::Plain(p) if p.name.as_str() == "slot" => match &p.value {
            Some(v) => match v.parts.first() {
                Some(svn_parser::AttrValuePart::Text { range }) => range.slice(source) != "default",
                _ => true,
            },
            None => true,
        },
        Attribute::Expression(e) => e.name.as_str() == "slot",
        Attribute::Shorthand(s) => s.name.as_str() == "slot",
        _ => false,
    })
}
