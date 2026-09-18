//! Rules that fire on regular DOM elements.

use smol_str::SmolStr;
use svn_parser::ast::{AttrValuePart, Attribute, DirectiveKind, Element};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::util::{is_mathml_element, is_svg_element, is_void_element};
use crate::walk::PathFrame;

/// Reference: html attribute name → correct JSX-like name (e.g.
/// `className` → `class`). Mirrors upstream
/// `phases/2-analyze/visitors/shared/element.js::react_attributes`.
const REACT_ATTRIBUTE_RENAMES: &[(&str, &str)] = &[("className", "class"), ("htmlFor", "for")];

/// Dispatch for regular DOM elements.
pub fn visit(
    el: &Element,
    ctx: &mut LintContext<'_>,
    parent_tag: Option<&str>,
    ancestors: &[crate::walk::Ancestor],
) {
    // `<slot>` is its own node type to the compiler (`SlotElement`),
    // which runs none of the regular-element checks below.
    if el.name == "slot" {
        visit_slot_element(el, ctx);
        return;
    }

    // `<title>` inside `<svelte:head>` is its own node type to the
    // compiler (`TitleElement`), with only its own two checks.
    if el.name == "title" && parent_is_head(&ctx.template_path) {
        visit_title_element(el, ctx);
        return;
    }

    validate_element_errors(&el.attributes, false, ctx);

    // A `<textarea>` gets its value from either its content or a
    // `value` attribute, not both (`RegularElement.js`).
    if el.name == "textarea"
        && !el.children.nodes.is_empty()
        && el.attributes.iter().any(|a| {
            matches!(
                a,
                Attribute::Plain(_) | Attribute::Expression(_) | Attribute::Shorthand(_)
            ) && attribute_name(a) == Some("value")
        })
    {
        ctx.emit_error(
            Code::textarea_invalid_content,
            messages::textarea_invalid_content(),
            el.range,
        );
    }

    // node_invalid_placement / node_invalid_placement_ssr
    // (`RegularElement.js`): walking out from the element, the parent
    // element is checked with `is_tag_valid_with_parent`, every
    // regular-element ancestor beyond it with
    // `is_tag_valid_with_ancestor` against the growing list, stopping
    // at a component / `<svelte:element>` / snippet. A control-flow
    // block passed on the way turns the error into the `_ssr`
    // warning (it renders as a separate template).
    if let Some(parent) = parent_tag {
        let mut past_parent = false;
        let mut only_warn = false;
        let mut list: Vec<&str> = vec![parent];
        let path = std::mem::take(&mut ctx.template_path);
        for frame in path.iter().rev() {
            if frame.is_block() {
                only_warn = true;
            }
            let message = match frame {
                PathFrame::RegularElement { name, .. } if !past_parent => {
                    if name != parent {
                        continue;
                    }
                    past_parent = true;
                    crate::html5::is_tag_valid_with_parent(el.name.as_str(), parent)
                }
                PathFrame::RegularElement { name, .. } => {
                    list.push(name.as_str());
                    crate::html5::is_tag_valid_with_ancestor(el.name.as_str(), &list)
                }
                PathFrame::Component { .. }
                | PathFrame::SvelteElement { .. }
                | PathFrame::SnippetBlock
                    if past_parent =>
                {
                    break;
                }
                _ => None,
            };
            if let Some(message) = message {
                if only_warn {
                    let full = messages::node_invalid_placement_ssr(&message);
                    ctx.emit(Code::node_invalid_placement_ssr, full, el.range);
                } else {
                    let full = messages::node_invalid_placement(&message);
                    ctx.emit_error(Code::node_invalid_placement, full, el.range);
                }
            }
        }
        ctx.template_path = path;
    }

    // component_name_lowercase: the tag starts lowercase AND resolves
    // to an import-kind binding with zero references in the script.
    // Upstream: `visitors/RegularElement.js:120-127` — resolved from
    // the element's own scope, so a template binding (an each-block
    // context, a `let:`) shadowing the import suppresses it.
    if let Some(tree) = &ctx.scope_tree
        && let Some(bid) = tree.resolve(
            tree.innermost_template_scope_at(el.range.start),
            el.name.as_str(),
        )
    {
        let b = tree.binding(bid);
        if b.declaration_kind == crate::scope::DeclarationKind::Import && b.references.is_empty() {
            let msg = messages::component_name_lowercase(el.name.as_str());
            ctx.emit(Code::component_name_lowercase, msg, el.range);
        }
    }

    // element_invalid_self_closing_tag.
    //
    // Upstream `RegularElement.js:217-223`: `source[node.end - 2]
    // === '/'` AND not void/svg/mathml. No structural self-closing
    // gate — the source peek is the definitive signal and handles
    // multiline `<video\n  …\n  />` where the structural parser
    // might flag the element differently.
    //
    // Upstream strips namespace prefix (anything ending in `:`)
    // before the void/svg/mathml lookup. So `enhanced:img` → `img`,
    // which is void → no warning.
    let bare_name = strip_tag_namespace(&el.name);
    // Upstream parses `<slot>` as a separate `SlotElement` node
    // type, so RegularElement's self-closing rule never sees it.
    // Our parser treats it as a regular Element; explicitly skip to
    // preserve byte parity.
    if bare_name != "slot"
        && !is_void_element(bare_name)
        && !is_svg_element(bare_name)
        && !is_mathml_element(bare_name)
    {
        let end = el.range.end as usize;
        if end >= 2 {
            let bytes = ctx.source.as_bytes();
            if bytes.get(end - 2) == Some(&b'/') {
                let msg = messages::element_invalid_self_closing_tag(el.name.as_str());
                ctx.emit(Code::element_invalid_self_closing_tag, msg, el.range);
            }
        }
    }

    let is_custom = crate::walk::is_custom_element_node(&el.name, &el.attributes);
    let parent = AttrParent::RegularElement {
        is_custom,
        name: el.name.as_str(),
    };

    // Per-attribute rules.
    for attr in &el.attributes {
        visit_attribute(attr, &el.attributes, ctx, parent);
    }

    // a11y dispatch — runs after the basic attribute rules because
    // upstream's `check_element` path sits in the analyze visitors
    // and depends on the collected attribute map.
    crate::rules::a11y_rules::visit_regular(el, ctx, ancestors);
}

/// `SlotElement.js`: the deprecation warning, then a static,
/// non-`default` `name` and nothing but attributes and `let:`
/// directives.
fn visit_slot_element(el: &Element, ctx: &mut LintContext<'_>) {
    // Not in runes mode's custom-element components
    // (`<svelte:options customElement>`).
    if ctx.runes && ctx.custom_element_info.is_none() {
        let msg = messages::slot_element_deprecated();
        ctx.emit(Code::slot_element_deprecated, msg, el.range);
    }
    record_slot_name(el, ctx);
    for attr in &el.attributes {
        match attr {
            Attribute::Plain(p) if p.name == "name" => match static_text_value(p, ctx.source) {
                None => ctx.emit_error(
                    Code::slot_element_invalid_name,
                    messages::slot_element_invalid_name(),
                    p.range,
                ),
                Some("default") => ctx.emit_error(
                    Code::slot_element_invalid_name_default,
                    messages::slot_element_invalid_name_default(),
                    p.range,
                ),
                Some(_) => {}
            },
            Attribute::Expression(e) if e.name == "name" => ctx.emit_error(
                Code::slot_element_invalid_name,
                messages::slot_element_invalid_name(),
                e.range,
            ),
            Attribute::Shorthand(s) if s.name == "name" => ctx.emit_error(
                Code::slot_element_invalid_name,
                messages::slot_element_invalid_name(),
                s.range,
            ),
            Attribute::Plain(_)
            | Attribute::Expression(_)
            | Attribute::Shorthand(_)
            | Attribute::Spread(_)
            | Attribute::Comment(_) => {}
            Attribute::Directive(d) if d.kind == DirectiveKind::Let => {}
            Attribute::Directive(d) => ctx.emit_error(
                Code::slot_element_invalid_attribute,
                messages::slot_element_invalid_attribute(),
                d.range,
            ),
        }
    }
}

/// `validate_slot_attribute` for a component's `slot` attribute, where
/// a misplaced `slot` is not an error.
pub(crate) fn validate_component_slot_attribute(attr: &Attribute, ctx: &mut LintContext<'_>) {
    match attr {
        Attribute::Plain(p) if p.name == "slot" => {
            let text = static_text_value(p, ctx.source);
            validate_slot_attribute(ctx, text, p.range, true);
        }
        Attribute::Shorthand(s) if s.name == "slot" => {
            validate_slot_attribute(ctx, None, s.range, true);
        }
        Attribute::Expression(e) if e.name == "slot" => {
            validate_slot_attribute(ctx, None, e.range, true);
        }
        _ => {}
    }
}

/// Record the slot this `<slot>` fills in the compiler's
/// `slot_names` map: keyed by name (`default` without a `name`
/// attribute), iterated in first-insertion order, holding the latest
/// element per name. Only the first entry is ever read. A non-static
/// name has already failed the component.
fn record_slot_name(el: &Element, ctx: &mut LintContext<'_>) {
    let mut name = "default";
    for attr in &el.attributes {
        if let Attribute::Plain(p) = attr
            && p.name == "name"
            && let Some(text) = static_text_value(p, ctx.source)
        {
            name = text;
        }
    }
    match &mut ctx.first_slot {
        None => ctx.first_slot = Some((SmolStr::new(name), el.range)),
        Some((first, range)) if first == name => *range = el.range,
        Some(_) => {}
    }
}

/// The value of an attribute that is exactly one text chunk
/// (`is_text_attribute`); an empty quoted value is an empty chunk.
fn static_text_value<'s>(p: &svn_parser::ast::PlainAttr, source: &'s str) -> Option<&'s str> {
    let value = p.value.as_ref()?;
    match value.parts.as_slice() {
        [] if value.quoted => Some(""),
        [AttrValuePart::Text { range }] => source.get(range.start as usize..range.end as usize),
        _ => None,
    }
}

/// The compiler's `regex_illegal_attribute_character`:
/// `/(^[0-9-.])|[\^$@%&#?!|()[\]{}^*+~;]/`.
fn is_illegal_attribute_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_digit() || c == '-' || c == '.')
        || name.contains([
            '^', '$', '@', '%', '&', '#', '?', '!', '|', '(', ')', '[', ']', '{', '}', '*', '+',
            '~', ';',
        ])
}

/// `validate_slot_attribute` (`shared/attribute.js`) for a `slot`
/// attribute on an element (`is_component == false`) or a component.
/// `text` is the static value when the attribute is a single text
/// chunk.
fn validate_slot_attribute(
    ctx: &mut LintContext<'_>,
    text: Option<&str>,
    range: svn_core::Range,
    is_component: bool,
) {
    let path = &mut ctx.template_path;
    // The node that owns the fragment the element sits in.
    let direct_child_of = path.last();
    // The nearest component-like or custom-element ancestor.
    let owner = path.iter().rposition(|f| {
        matches!(
            f,
            PathFrame::Component { .. }
                | PathFrame::SvelteElement { .. }
                | PathFrame::RegularElement { custom: true, .. }
        )
    });
    let invalid_value = text.is_none();
    let error = if direct_child_of == Some(&PathFrame::SnippetBlock) {
        invalid_value.then_some(SlotError::InvalidValue)
    } else {
        match owner.map(|i| (i, &path[i])) {
            Some((i, PathFrame::Component { .. })) if i + 1 != path.len() => {
                (!is_component).then_some(SlotError::InvalidPlacement)
            }
            Some((i, PathFrame::Component { .. })) => match (text, &mut path[i]) {
                (None, _) => Some(SlotError::InvalidValue),
                (
                    Some(name),
                    PathFrame::Component {
                        name: owner,
                        default_slot_content,
                        filled_slots,
                        ..
                    },
                ) => {
                    if filled_slots.iter().any(|s| s == name) {
                        Some(SlotError::Duplicate(SmolStr::new(name), owner.clone()))
                    } else {
                        filled_slots.push(SmolStr::new(name));
                        match default_slot_content {
                            Some(content) if name == "default" => {
                                Some(SlotError::DefaultDuplicate(*content))
                            }
                            _ => None,
                        }
                    }
                }
                (Some(_), _) => None,
            },
            Some(_) => None,
            None => (!is_component).then_some(SlotError::InvalidPlacement),
        }
    };
    match error {
        Some(SlotError::InvalidValue) => ctx.emit_error(
            Code::slot_attribute_invalid,
            messages::slot_attribute_invalid(),
            range,
        ),
        Some(SlotError::InvalidPlacement) => ctx.emit_error(
            Code::slot_attribute_invalid_placement,
            messages::slot_attribute_invalid_placement(),
            range,
        ),
        Some(SlotError::Duplicate(name, owner)) => ctx.emit_error(
            Code::slot_attribute_duplicate,
            messages::slot_attribute_duplicate(&name, &owner),
            range,
        ),
        Some(SlotError::DefaultDuplicate(content)) => ctx.emit_error(
            Code::slot_default_duplicate,
            messages::slot_default_duplicate(),
            content,
        ),
        None => {}
    }
}

enum SlotError {
    InvalidValue,
    InvalidPlacement,
    /// A second child filling the same slot of the component.
    Duplicate(SmolStr, SmolStr),
    /// `slot="default"` next to other default-slot content (its range).
    DefaultDuplicate(svn_core::Range),
}

/// Parent kinds understood by the shared attribute visitor. Drives
/// rule dispatch (e.g. `attribute_quoted` only on Component/
/// SvelteComponent/SvelteSelf/custom-element;
/// `event_directive_deprecated` only on RegularElement/SvelteElement).
#[derive(Clone, Copy)]
pub(crate) enum AttrParent<'a> {
    /// A regular HTML element.
    RegularElement { is_custom: bool, name: &'a str },
    /// A `<Component>` invocation.
    Component,
    /// `<svelte:component>` or `<svelte:self>`.
    SvelteComponentLike,
    /// `<svelte:element>` — dynamic element.
    SvelteElement,
    /// `<svelte:window>` / `<svelte:document>` / `<svelte:body>` — the
    /// special elements the compiler validates bindings on, by name.
    SvelteSpecial(&'static str),
    /// `<svelte:fragment>`.
    SvelteFragment,
    /// Any other `<svelte:*>` (options/head/boundary).
    OtherSvelte,
}

impl<'a> AttrParent<'a> {
    /// The name the compiler's binding table is checked against.
    fn binding_target_name(self) -> Option<&'a str> {
        match self {
            Self::RegularElement { name, .. } => Some(name),
            Self::SvelteElement => Some("svelte:element"),
            Self::SvelteSpecial(name) => Some(name),
            _ => None,
        }
    }

    fn is_quotable(self) -> bool {
        matches!(
            self,
            Self::Component
                | Self::SvelteComponentLike
                | Self::RegularElement {
                    is_custom: true,
                    ..
                }
        )
    }
    fn fires_event_directive_deprecated(self) -> bool {
        // Upstream OnDirective.js:16 only fires on RegularElement /
        // SvelteElement parents; Components / SvelteComponent /
        // SvelteSelf are excluded so user can't be blamed for a
        // library's unconverted `on:click` event forwarding.
        matches!(self, Self::RegularElement { .. } | Self::SvelteElement)
    }
}

/// `siblings` is the owning node's whole attribute list, which some
/// `bind:` checks consult.
pub(crate) fn visit_attribute(
    attr: &Attribute,
    siblings: &[Attribute],
    ctx: &mut LintContext<'_>,
    parent: AttrParent<'_>,
) {
    let parent_is_quotable = parent.is_quotable();
    let fires_event_directive = parent.fires_event_directive_deprecated();
    let parent_is_regular_or_svelte = matches!(
        parent,
        AttrParent::RegularElement { .. } | AttrParent::SvelteElement
    );
    let fires_invalid_property_name = parent_is_regular_or_svelte;
    let fires_attr_name_checks =
        !matches!(parent, AttrParent::OtherSvelte | AttrParent::SvelteFragment);
    // `on*` attributes / `on:` directives on elements, for
    // `mixed_event_handler_syntaxes`.
    if parent_is_regular_or_svelte {
        match attr {
            Attribute::Plain(p)
                if p.name.starts_with("on")
                    && matches!(
                        p.value.as_ref().map(|v| v.parts.as_slice()),
                        Some([AttrValuePart::Expression { .. }])
                    ) =>
            {
                ctx.uses_event_attributes = true;
            }
            Attribute::Expression(e) if e.name.starts_with("on") => {
                ctx.uses_event_attributes = true;
            }
            Attribute::Shorthand(s) if s.name.starts_with("on") => {
                ctx.uses_event_attributes = true;
            }
            Attribute::Directive(d) if d.kind == DirectiveKind::On => {
                if ctx.event_directive.is_none() {
                    ctx.event_directive = Some((d.name.clone(), d.range));
                }
            }
            _ => {}
        }
    }
    match attr {
        Attribute::Plain(p) => {
            let name = p.name.as_str();

            // attribute_illegal_colon — upstream:
            // `attr.name.includes(':')` AND NOT xmlns:/xlink:/xml:
            if fires_attr_name_checks
                && name.contains(':')
                && !name.starts_with("xmlns:")
                && !name.starts_with("xlink:")
                && !name.starts_with("xml:")
            {
                let msg = messages::attribute_illegal_colon();
                ctx.emit(Code::attribute_illegal_colon, msg, p.range);
            }

            // attribute_avoid_is: `is="..."` on any element.
            if parent_is_regular_or_svelte && name == "is" {
                let msg = messages::attribute_avoid_is();
                ctx.emit(Code::attribute_avoid_is, msg, p.range);
            }

            // attribute_invalid_property_name: React-style name.
            if fires_invalid_property_name
                && let Some(correct) = REACT_ATTRIBUTE_RENAMES
                    .iter()
                    .find_map(|(wrong, right)| if *wrong == name { Some(*right) } else { None })
            {
                let msg = messages::attribute_invalid_property_name(name, correct);
                ctx.emit(Code::attribute_invalid_property_name, msg, p.range);
            }

            // attribute_quoted: runes-mode + single-expression value
            // + parent is Component / SvelteComponent / SvelteSelf /
            // custom-element. The AttrValue carries a `quoted` flag.
            if ctx.runes
                && parent_is_quotable
                && let Some(v) = &p.value
                && v.quoted
                && v.parts.len() == 1
                && matches!(v.parts[0], AttrValuePart::Expression { .. })
            {
                let msg = messages::attribute_quoted();
                ctx.emit(Code::attribute_quoted, msg, p.range);
            }

            // An `on*` attribute holding a single expression (a quoted
            // `"{handler}"` is one) may name the global handler.
            if parent_is_regular_or_svelte
                && name.starts_with("on")
                && name.len() > 2
                && let Some(
                    [
                        AttrValuePart::Expression {
                            expression_range, ..
                        },
                    ],
                ) = p.value.as_ref().map(|v| v.parts.as_slice())
            {
                global_event_reference(name, *expression_range, p.range, ctx);
            }
        }
        Attribute::Shorthand(s) => {
            let name = s.name.as_str();
            if fires_invalid_property_name
                && let Some(correct) = REACT_ATTRIBUTE_RENAMES
                    .iter()
                    .find_map(|(wrong, right)| if *wrong == name { Some(*right) } else { None })
            {
                let msg = messages::attribute_invalid_property_name(name, correct);
                ctx.emit(Code::attribute_invalid_property_name, msg, s.range);
            }

            // `{onkeydown}` — name IS the value identifier. Fire iff
            // the name starts with `on` and has no local binding in
            // the instance/module scope.
            if parent_is_regular_or_svelte
                && name.starts_with("on")
                && name.len() > 2
                && let Some(tree) = &ctx.scope_tree
                && tree
                    .resolve(tree.innermost_template_scope_at(s.range.start), name)
                    .is_none()
            {
                let msg = messages::attribute_global_event_reference(name);
                ctx.emit(Code::attribute_global_event_reference, msg, s.range);
            }
        }
        Attribute::Expression(e) => {
            let name = e.name.as_str();
            if fires_attr_name_checks
                && name.contains(':')
                && !name.starts_with("xmlns:")
                && !name.starts_with("xlink:")
                && !name.starts_with("xml:")
            {
                let msg = messages::attribute_illegal_colon();
                ctx.emit(Code::attribute_illegal_colon, msg, e.range);
            }
            if fires_invalid_property_name
                && let Some(correct) = REACT_ATTRIBUTE_RENAMES
                    .iter()
                    .find_map(|(wrong, right)| if *wrong == name { Some(*right) } else { None })
            {
                let msg = messages::attribute_invalid_property_name(name, correct);
                ctx.emit(Code::attribute_invalid_property_name, msg, e.range);
            }
            if parent_is_regular_or_svelte && name.starts_with("on") && name.len() > 2 {
                global_event_reference(name, e.expression_range, e.range, ctx);
            }
        }
        Attribute::Directive(d) => {
            // `let:` belongs on a component, an element, a `<slot>` or
            // a `<svelte:fragment>` (`LetDirective.js`).
            if d.kind == DirectiveKind::Let
                && matches!(
                    parent,
                    AttrParent::OtherSvelte | AttrParent::SvelteSpecial(_)
                )
            {
                ctx.emit_error(
                    Code::let_directive_invalid_placement,
                    messages::let_directive_invalid_placement(),
                    d.range,
                );
            }
            // `style:` takes no modifier but `important` (`StyleDirective.js`).
            if d.kind == DirectiveKind::Style
                && (d.modifiers.len() > 1 || d.modifiers.iter().any(|m| m != "important"))
            {
                ctx.emit_error(
                    Code::style_directive_invalid_modifier,
                    messages::style_directive_invalid_modifier(),
                    d.range,
                );
            }
            // event_directive_deprecated: on:click in runes mode on a
            // regular DOM element OR `<svelte:element>` (but NOT
            // Components / svelte:component / svelte:self).
            if ctx.runes && fires_event_directive && d.kind == DirectiveKind::On {
                let msg = messages::event_directive_deprecated(d.name.as_str());
                ctx.emit(Code::event_directive_deprecated, msg, d.range);
            }

            // bind_invalid_each_rest — upstream `BindDirective.js`:
            // the bind expression's BASE identifier resolves to an
            // each-context binding declared inside a rest element.
            // Fires per bind site, anchored at the binding's
            // declaration, DURING the template walk — so the live
            // ignore stack applies and a template
            // `<!-- svelte-ignore … -->` above the `{#each}`
            // suppresses (verified against the compiler). Getter/
            // setter pairs (SequenceExpression) skip the binding
            // resolution upstream, so they don't fire.
            if d.kind == DirectiveKind::Bind {
                // The compiler checks the binding's name, then the
                // write it performs; writes in earlier attributes came
                // first.
                crate::rules::binding_rules::flush_template_write_violations(
                    ctx,
                    d.range.start,
                    false,
                );
                if let Some(target) = parent.binding_target_name() {
                    bind_name_checks(d, target, ctx);
                    if crate::rules::bind_properties::lookup(d.name.as_str()).is_some() {
                        bind_element_checks(d, target, siblings, ctx);
                    }
                }
                let expression = bind_expression(d);
                let is_sequence =
                    expression.is_some_and(|range| bind_sequence_checks(d, range, ctx.source, ctx));
                crate::rules::binding_rules::flush_template_write_violations(
                    ctx,
                    d.range.start,
                    true,
                );
                if !is_sequence {
                    if let Some(range) = expression
                        && !has_object_identifier(range, ctx.source)
                    {
                        ctx.emit_error(
                            Code::bind_invalid_expression,
                            messages::bind_invalid_expression(),
                            d.range,
                        );
                    }
                    if d.name != "this" {
                        bind_value_check(d, ctx);
                    }
                    if d.name == "group" {
                        bind_group_snippet_check(d, expression, ctx);
                    }
                }
            }
            if d.kind == DirectiveKind::Bind {
                use svn_parser::ast::DirectiveValue;
                let base: Option<String> = match &d.value {
                    Some(DirectiveValue::Expression {
                        expression_range, ..
                    }) => ctx
                        .source
                        .get(expression_range.start as usize..expression_range.end as usize)
                        .and_then(crate::scope_util::base_identifier_of_text),
                    // The parser reads `bind:x="{y}"` as `bind:x={y}`.
                    Some(DirectiveValue::Quoted(v)) => match v.parts.as_slice() {
                        [
                            AttrValuePart::Expression {
                                expression_range, ..
                            },
                        ] => ctx
                            .source
                            .get(expression_range.start as usize..expression_range.end as usize)
                            .and_then(crate::scope_util::base_identifier_of_text),
                        _ => None,
                    },
                    // `bind:foo` shorthand — the implied identifier.
                    None => Some(d.name.to_string()),
                    _ => None,
                };
                if let Some(base) = base.as_deref()
                    && let Some(tree) = &ctx.scope_tree
                    && let Some(bid) =
                        tree.resolve(tree.innermost_template_scope_at(d.range.start), base)
                {
                    let b = tree.binding(bid);
                    if b.kind == crate::scope::BindingKind::Each && b.inside_rest {
                        let msg = messages::bind_invalid_each_rest(b.name.as_str());
                        let range = b.range;
                        ctx.emit(Code::bind_invalid_each_rest, msg, range);
                    }
                }
            }
        }
        _ => {}
    }
}

/// `bind_invalid_value` (`BindDirective.js`): binding an identifier
/// that is neither state, a prop, an each item, a store nor otherwise
/// written to — in practice, one that resolves to no binding at all
/// (every binding a `bind:` names counts as written).
fn bind_value_check(d: &svn_parser::ast::Directive, ctx: &mut LintContext<'_>) {
    use crate::scope::BindingKind;
    use svn_parser::ast::DirectiveValue;
    let expression = match &d.value {
        Some(DirectiveValue::Expression {
            expression_range, ..
        }) => Some(*expression_range),
        Some(DirectiveValue::Quoted(v)) => match v.parts.as_slice() {
            [
                AttrValuePart::Expression {
                    expression_range, ..
                },
            ] => Some(*expression_range),
            _ => None,
        },
        None => None,
        _ => return,
    };
    let (name, range) = match expression {
        Some(r) => {
            let Some(text) = ctx.source.get(r.start as usize..r.end as usize) else {
                return;
            };
            let Some((name, start, end)) = crate::scope_util::identifier_of_text(text) else {
                return;
            };
            (name, svn_core::Range::new(r.start + start, r.start + end))
        }
        // `bind:name` names the identifier it binds.
        None => {
            let start = d.range.start + "bind:".len() as u32;
            (
                d.name.to_string(),
                svn_core::Range::new(start, start + d.name.len() as u32),
            )
        }
    };
    let Some(tree) = &ctx.scope_tree else {
        return;
    };
    let scope = tree.innermost_template_scope_at(d.range.start);
    let valid = tree.resolve(scope, &name).is_some_and(|bid| {
        let b = tree.binding(bid);
        matches!(
            b.kind,
            BindingKind::State
                | BindingKind::RawState
                | BindingKind::Prop
                | BindingKind::BindableProp
                | BindingKind::Each
                | BindingKind::StoreSub
        ) || b.reassigned
            || b.mutated
    });
    if !valid {
        ctx.emit_error(
            Code::bind_invalid_value,
            messages::bind_invalid_value(),
            range,
        );
    }
}

/// `attribute_global_event_reference` (upstream `shared/element.js`):
/// an `on*` attribute whose expression is the bare identifier of the
/// same name, with no binding in scope — the global handler property.
/// The expression is compared as the compiler's AST sees it, so
/// parentheses and type wrappers do not count.
fn global_event_reference(
    name: &str,
    expression: svn_core::Range,
    attr: svn_core::Range,
    ctx: &mut LintContext<'_>,
) {
    use oxc_ast::ast::{Expression, Statement};
    let Some(src) = ctx
        .source
        .get(expression.start as usize..expression.end as usize)
    else {
        return;
    };
    let wrapped = format!("({src});");
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let Some(Statement::ExpressionStatement(stmt)) = parsed.program.body.first() else {
        return;
    };
    let mut expr = &stmt.expression;
    loop {
        expr = match expr {
            Expression::ParenthesizedExpression(p) => &p.expression,
            Expression::TSAsExpression(e) => &e.expression,
            Expression::TSSatisfiesExpression(e) => &e.expression,
            Expression::TSNonNullExpression(e) => &e.expression,
            Expression::TSTypeAssertion(e) => &e.expression,
            _ => break,
        };
    }
    let Expression::Identifier(id) = expr else {
        return;
    };
    if id.name != name {
        return;
    }
    if let Some(tree) = &ctx.scope_tree
        && tree
            .resolve(tree.innermost_template_scope_at(attr.start), name)
            .is_none()
    {
        let msg = messages::attribute_global_event_reference(name);
        ctx.emit(Code::attribute_global_event_reference, msg, attr);
    }
}

/// Strip namespace prefix from tag name: `enhanced:img` → `img`,
/// `foo:bar:baz` → `baz`. Mirrors upstream regex
/// `node.name.replace(/[a-zA-Z-]*:/g, '')`.
fn strip_tag_namespace(name: &str) -> &str {
    match name.rfind(':') {
        Some(i) => &name[i + 1..],
        None => name,
    }
}

/// Compiler errors `bind_invalid_target` / `bind_invalid_name` —
/// upstream `BindDirective.js`: a known binding used on an element
/// outside its `valid_elements` (or inside its `invalid_elements`), or
/// a name the table does not know, with a fuzzy "Did you mean" when a
/// close name exists that is allowed on this element.
fn bind_name_checks(d: &svn_parser::ast::Directive, target: &str, ctx: &mut LintContext<'_>) {
    use crate::rules::bind_properties::{BINDING_PROPERTIES, allowed_on, lookup};
    let name = d.name.as_str();
    match lookup(name) {
        Some(property) => {
            if !property.valid_elements.is_empty() && !property.valid_elements.contains(&target) {
                let elements: Vec<String> = property
                    .valid_elements
                    .iter()
                    .map(|e| format!("`<{e}>`"))
                    .collect();
                ctx.emit_error(
                    Code::bind_invalid_target,
                    messages::bind_invalid_target(name, &elements.join(", ")),
                    d.range,
                );
            }
            if property.invalid_elements.contains(&target) {
                let mut valid: Vec<&str> = BINDING_PROPERTIES
                    .iter()
                    .filter(|p| allowed_on(p, target))
                    .map(|p| p.name)
                    .collect();
                valid.sort_unstable();
                ctx.emit_error(
                    Code::bind_invalid_name,
                    messages::bind_invalid_name(
                        name,
                        Some(&format!(
                            "Possible bindings for <{target}> are {}",
                            valid.join(", ")
                        )),
                    ),
                    d.range,
                );
            }
        }
        None => {
            let names: Vec<&str> = BINDING_PROPERTIES.iter().map(|p| p.name).collect();
            if let Some(matched) = crate::fuzzymatch::fuzzymatch(name, &names)
                && let Some(property) = lookup(matched)
                && (property.valid_elements.is_empty() || property.valid_elements.contains(&target))
            {
                ctx.emit_error(
                    Code::bind_invalid_name,
                    messages::bind_invalid_name(name, Some(&format!("Did you mean '{matched}'?"))),
                    d.range,
                );
            }
            ctx.emit_error(
                Code::bind_invalid_name,
                messages::bind_invalid_name(name, None),
                d.range,
            );
        }
    }
}

/// The event modifiers the compiler knows (`shared/element.js`).
const EVENT_MODIFIERS: &[&str] = &[
    "preventDefault",
    "stopPropagation",
    "stopImmediatePropagation",
    "capture",
    "once",
    "passive",
    "nonpassive",
    "self",
    "trusted",
];

/// The errors of the compiler's `validate_element`, which runs over a
/// regular element's or a `<svelte:element>`'s attributes before
/// anything else the element's visitor checks — so these errors
/// precede placement errors on the element and the errors its
/// attributes raise when visited one by one.
///
/// `skip_this` drops the first `this` attribute of a
/// `<svelte:element>`, which the parser moves out of the attribute
/// list.
pub(crate) fn validate_element_errors(
    attributes: &[Attribute],
    skip_this: bool,
    ctx: &mut LintContext<'_>,
) {
    let mut has_animate = false;
    let mut in_transition: Option<DirectiveKind> = None;
    let mut out_transition: Option<DirectiveKind> = None;
    let mut this_skipped = !skip_this;
    for attr in attributes {
        if !this_skipped && attribute_name(attr) == Some("this") {
            this_skipped = true;
            continue;
        }
        match attr {
            Attribute::Plain(_) | Attribute::Expression(_) | Attribute::Shorthand(_) => {
                validate_attribute_errors(attr, ctx);
            }
            Attribute::Directive(d) => match d.kind {
                DirectiveKind::Animate => {
                    let error = match ctx.template_path.last() {
                        Some(PathFrame::EachBlock { has_key: false, .. }) => Some((
                            Code::animation_missing_key,
                            messages::animation_missing_key(),
                        )),
                        Some(PathFrame::EachBlock { body_nodes, .. }) if *body_nodes <= 1 => None,
                        _ => Some((
                            Code::animation_invalid_placement,
                            messages::animation_invalid_placement(),
                        )),
                    };
                    if let Some((code, message)) = error {
                        ctx.emit_error(code, message, d.range);
                    }
                    if has_animate {
                        ctx.emit_error(
                            Code::animation_duplicate,
                            messages::animation_duplicate(),
                            d.range,
                        );
                    }
                    has_animate = true;
                }
                DirectiveKind::Transition | DirectiveKind::In | DirectiveKind::Out => {
                    let intro = d.kind != DirectiveKind::Out;
                    let outro = d.kind != DirectiveKind::In;
                    let existing = if intro && in_transition.is_some() {
                        in_transition
                    } else if outro {
                        out_transition
                    } else {
                        None
                    };
                    if let Some(existing) = existing {
                        let a = existing.as_str();
                        let b = d.kind.as_str();
                        if a == b {
                            ctx.emit_error(
                                Code::transition_duplicate,
                                messages::transition_duplicate(a),
                                d.range,
                            );
                        } else {
                            ctx.emit_error(
                                Code::transition_conflict,
                                messages::transition_conflict(a, b),
                                d.range,
                            );
                        }
                    }
                    if intro {
                        in_transition = Some(d.kind);
                    }
                    if outro {
                        out_transition = Some(d.kind);
                    }
                }
                DirectiveKind::On => {
                    let mut passive = false;
                    let mut conflicting: Option<&str> = None;
                    for modifier in &d.modifiers {
                        if !EVENT_MODIFIERS.contains(&modifier.as_str()) {
                            let list = format!(
                                "{} or {}",
                                EVENT_MODIFIERS[..EVENT_MODIFIERS.len() - 1].join(", "),
                                EVENT_MODIFIERS[EVENT_MODIFIERS.len() - 1]
                            );
                            ctx.emit_error(
                                Code::event_handler_invalid_modifier,
                                messages::event_handler_invalid_modifier(&list),
                                d.range,
                            );
                        }
                        if modifier == "passive" {
                            passive = true;
                        } else if modifier == "nonpassive" || modifier == "preventDefault" {
                            conflicting = Some(modifier.as_str());
                        }
                        if passive && let Some(other) = conflicting {
                            ctx.emit_error(
                                Code::event_handler_invalid_modifier_combination,
                                messages::event_handler_invalid_modifier_combination(
                                    "passive", other,
                                ),
                                d.range,
                            );
                        }
                    }
                }
                _ => {}
            },
            Attribute::Spread(_) | Attribute::Comment(_) => {}
        }
    }
}

/// The error half of `validate_element`'s per-attribute checks, in the
/// compiler's order: the runes-mode value-shape checks, an illegal
/// name, an `on*` attribute that is not a single expression, and the
/// `slot` attribute's placement.
fn validate_attribute_errors(attr: &Attribute, ctx: &mut LintContext<'_>) {
    let (name, range) = match attr {
        Attribute::Plain(p) => (p.name.as_str(), p.range),
        Attribute::Expression(e) => (e.name.as_str(), e.range),
        Attribute::Shorthand(s) => (s.name.as_str(), s.range),
        _ => return,
    };
    let expression = match attr {
        Attribute::Plain(p) => match p.value.as_ref().map(|v| v.parts.as_slice()) {
            Some(
                [
                    AttrValuePart::Expression {
                        expression_range, ..
                    },
                ],
            ) => Some(*expression_range),
            _ => None,
        },
        Attribute::Expression(e) => Some(e.expression_range),
        Attribute::Shorthand(s) => Some(svn_core::Range::new(s.range.start + 1, s.range.end - 1)),
        _ => None,
    };
    if ctx.runes {
        attribute_value_shape_errors(attr, expression, ctx);
    }
    if is_illegal_attribute_name(name) {
        ctx.emit_error(
            Code::attribute_invalid_name,
            messages::attribute_invalid_name(name),
            range,
        );
    }
    if name.starts_with("on") && name.len() > 2 && expression.is_none() {
        ctx.emit_error(
            Code::attribute_invalid_event_handler,
            messages::attribute_invalid_event_handler(),
            range,
        );
    }
    if name == "slot" {
        let text = match attr {
            Attribute::Plain(p) => static_text_value(p, ctx.source),
            _ => None,
        };
        validate_slot_attribute(ctx, text, range, false);
    }
}

/// The runes-mode value checks of an element's or component's
/// attribute: a value of several chunks must be quoted
/// (`validate_attribute`), and a single expression may not be an
/// unparenthesized comma sequence.
fn attribute_value_shape_errors(
    attr: &Attribute,
    expression: Option<svn_core::Range>,
    ctx: &mut LintContext<'_>,
) {
    if let Attribute::Plain(p) = attr
        && let Some(v) = &p.value
        && v.parts.len() > 1
        && !v.quoted
    {
        ctx.emit_error(
            Code::attribute_unquoted_sequence,
            messages::attribute_unquoted_sequence(),
            p.range,
        );
    }
    if let Some(expr) = expression {
        sequence_expression_error(expr, ctx);
    }
}

/// `attribute_invalid_sequence_expression` for the expression in `range`.
pub(crate) fn sequence_expression_error(range: svn_core::Range, ctx: &mut LintContext<'_>) {
    if let Some(sequence) = unparenthesized_sequence(range, ctx.source) {
        ctx.emit_error(
            Code::attribute_invalid_sequence_expression,
            messages::attribute_invalid_sequence_expression(),
            sequence,
        );
    }
}

/// The runes-mode value checks of a component attribute (see
/// [`attribute_value_shape_errors`]).
pub(crate) fn component_attribute_value_errors(attr: &Attribute, ctx: &mut LintContext<'_>) {
    let expression = match attr {
        Attribute::Plain(p) => match p.value.as_ref().map(|v| v.parts.as_slice()) {
            Some(
                [
                    AttrValuePart::Expression {
                        expression_range, ..
                    },
                ],
            ) => Some(*expression_range),
            _ => None,
        },
        Attribute::Expression(e) => Some(e.expression_range),
        Attribute::Shorthand(_) => None,
        _ => return,
    };
    attribute_value_shape_errors(attr, expression, ctx);
}

/// The range of the expression in `range` when it is a comma sequence
/// the compiler rejects as an attribute value: scanning back from the
/// sequence's first character, an opening `{` comes before any `(`.
fn unparenthesized_sequence(range: svn_core::Range, source: &str) -> Option<svn_core::Range> {
    use oxc_ast::ast::Expression;
    let text = source.get(range.start as usize..range.end as usize)?;
    let alloc = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let parsed = oxc_parser::Parser::new(&alloc, text, source_type)
        .parse_expression()
        .ok()?;
    let Expression::SequenceExpression(seq) = &parsed else {
        return None;
    };
    let start = range.start + seq.span.start;
    let bytes = source.as_bytes();
    let mut i = start as usize;
    while i > 1 {
        i -= 1;
        match bytes[i] {
            b'(' => return None,
            b'{' => return Some(svn_core::Range::new(start, range.start + seq.span.end)),
            _ => {}
        }
    }
    None
}

/// The name of a plain, expression or shorthand attribute.
fn attribute_name(attr: &Attribute) -> Option<&str> {
    match attr {
        Attribute::Plain(p) => Some(p.name.as_str()),
        Attribute::Expression(e) => Some(e.name.as_str()),
        Attribute::Shorthand(s) => Some(s.name.as_str()),
        _ => None,
    }
}

/// Whether an attribute is a single static text chunk (the compiler's
/// `is_text_attribute`), and that text.
fn text_attribute<'s>(attr: &Attribute, source: &'s str) -> Option<&'s str> {
    match attr {
        Attribute::Plain(p) => static_text_value(p, source),
        _ => None,
    }
}

/// Whether an attribute is the bare, valueless form (`value === true`).
fn is_bare_attribute(attr: &Attribute) -> bool {
    matches!(attr, Attribute::Plain(p) if p.value.is_none())
}

/// The first plain, expression or shorthand attribute called `name`.
fn find_attribute<'a>(attributes: &'a [Attribute], name: &str) -> Option<&'a Attribute> {
    attributes.iter().find(|a| attribute_name(a) == Some(name))
}

/// The element-specific `bind:` checks of `BindDirective.js` for a
/// binding the compiler's table knows: `<input>` needs a static `type`
/// (and the right one for `checked` / `files`), `<select>` a static
/// `multiple`, `offsetWidth` is not for SVG, and the content-editable
/// bindings need a static `contenteditable`.
fn bind_element_checks(
    d: &svn_parser::ast::Directive,
    target: &str,
    siblings: &[Attribute],
    ctx: &mut LintContext<'_>,
) {
    let name = d.name.as_str();
    if target == "input" && name != "this" {
        let type_attr = find_attribute(siblings, "type");
        match type_attr {
            Some(t) if text_attribute(t, ctx.source).is_none() => {
                if name != "value" || is_bare_attribute(t) {
                    ctx.emit_error(
                        Code::attribute_invalid_type,
                        messages::attribute_invalid_type(),
                        t.range(),
                    );
                }
            }
            _ => {
                let type_text = type_attr.and_then(|t| text_attribute(t, ctx.source));
                if name == "checked" && type_text != Some("checkbox") {
                    let radio = if type_text == Some("radio") {
                        " — for `<input type=\"radio\">`, use `bind:group`"
                    } else {
                        ""
                    };
                    ctx.emit_error(
                        Code::bind_invalid_target,
                        messages::bind_invalid_target(
                            name,
                            &format!("`<input type=\"checkbox\">`{radio}"),
                        ),
                        d.range,
                    );
                }
                if name == "files" && type_text != Some("file") {
                    ctx.emit_error(
                        Code::bind_invalid_target,
                        messages::bind_invalid_target(name, "`<input type=\"file\">`"),
                        d.range,
                    );
                }
            }
        }
    }
    if target == "select"
        && name != "this"
        && let Some(multiple) = siblings.iter().find(|a| {
            attribute_name(a) == Some("multiple")
                && text_attribute(a, ctx.source).is_none()
                && !is_bare_attribute(a)
        })
    {
        ctx.emit_error(
            Code::attribute_invalid_multiple,
            messages::attribute_invalid_multiple(),
            multiple.range(),
        );
    }
    if name == "offsetWidth" && is_svg_element(target) {
        ctx.emit_error(
            Code::bind_invalid_target,
            messages::bind_invalid_target(
                name,
                "non-`<svg>` elements. Use `bind:clientWidth` for `<svg>` instead",
            ),
            d.range,
        );
    }
    if matches!(name, "textContent" | "innerHTML" | "innerText") {
        match find_attribute(siblings, "contenteditable") {
            None => ctx.emit_error(
                Code::attribute_contenteditable_missing,
                messages::attribute_contenteditable_missing(),
                d.range,
            ),
            Some(c) if text_attribute(c, ctx.source).is_none() && !is_bare_attribute(c) => {
                ctx.emit_error(
                    Code::attribute_contenteditable_dynamic,
                    messages::attribute_contenteditable_dynamic(),
                    c.range(),
                );
            }
            Some(_) => {}
        }
    }
}

/// The source range of a `bind:` directive's expression; the
/// `bind:name` shorthand has none.
fn bind_expression(d: &svn_parser::ast::Directive) -> Option<svn_core::Range> {
    use svn_parser::ast::DirectiveValue;
    match &d.value {
        Some(DirectiveValue::Expression {
            expression_range, ..
        }) => Some(*expression_range),
        Some(DirectiveValue::BindPair {
            getter_range,
            setter_range,
            ..
        }) => Some(svn_core::Range::new(getter_range.start, setter_range.end)),
        Some(DirectiveValue::Quoted(_)) | None => None,
    }
}

/// Parse a template expression the way the compiler reads it.
fn with_parsed_expression<R>(
    range: svn_core::Range,
    source: &str,
    f: impl FnOnce(&oxc_ast::ast::Expression<'_>) -> R,
) -> Option<R> {
    let text = source.get(range.start as usize..range.end as usize)?;
    let alloc = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let parsed = oxc_parser::Parser::new(&alloc, text, source_type)
        .parse_expression()
        .ok()?;
    Some(f(&parsed))
}

/// Strip the parentheses the compiler's parser does not keep.
fn unparenthesized<'a, 'b>(
    mut expr: &'a oxc_ast::ast::Expression<'b>,
) -> &'a oxc_ast::ast::Expression<'b> {
    while let oxc_ast::ast::Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }
    expr
}

/// The getter/setter checks of `BindDirective.js` when the bound
/// expression is a comma sequence: `bind:group` cannot take one, the
/// pair may not be wrapped in parentheses, and it must have exactly two
/// members. Returns whether the expression is a sequence, in which case
/// the compiler skips the remaining binding checks.
fn bind_sequence_checks(
    d: &svn_parser::ast::Directive,
    range: svn_core::Range,
    source: &str,
    ctx: &mut LintContext<'_>,
) -> bool {
    let sequence = with_parsed_expression(range, source, |expr| match unparenthesized(expr) {
        oxc_ast::ast::Expression::SequenceExpression(seq) => {
            Some((range.start + seq.span.start, seq.expressions.len()))
        }
        _ => None,
    })
    .flatten();
    let Some((start, len)) = sequence else {
        return false;
    };
    if d.name == "group" {
        ctx.emit_error(
            Code::bind_group_invalid_expression,
            messages::bind_group_invalid_expression(),
            d.range,
        );
    }
    if parenthesis_before(source, start) {
        ctx.emit_error(
            Code::bind_invalid_parens,
            messages::bind_invalid_parens(d.name.as_str()),
            d.range,
        );
    }
    if len != 2 {
        ctx.emit_error(
            Code::bind_invalid_expression,
            messages::bind_invalid_expression(),
            d.range,
        );
    }
    true
}

/// Whether a `(` outside a comment sits between the `{` opening the
/// directive value and `start`.
fn parenthesis_before(source: &str, start: u32) -> bool {
    let Some(open) = source[..start as usize].rfind('{') else {
        return false;
    };
    let between = &source[open + 1..start as usize];
    let bytes = between.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i..].starts_with(b"/*") {
            i = between[i + 2..]
                .find("*/")
                .map_or(bytes.len(), |e| i + 2 + e + 2);
            continue;
        }
        if bytes[i..].starts_with(b"//") {
            i = between[i..].find('\n').map_or(bytes.len(), |e| i + e);
            continue;
        }
        if bytes[i] == b'(' {
            return true;
        }
        i += 1;
    }
    false
}

/// The compiler's `object()`: the identifier at the root of a
/// member-access chain; an expression of any other shape has none.
/// TypeScript wrappers are removed before analysis, so they don't
/// count.
fn has_object_identifier(range: svn_core::Range, source: &str) -> bool {
    use oxc_ast::ast::Expression;
    with_parsed_expression(range, source, |expr| {
        let mut expr = expr;
        loop {
            expr = match expr {
                Expression::ParenthesizedExpression(p) => &p.expression,
                Expression::TSAsExpression(e) => &e.expression,
                Expression::TSSatisfiesExpression(e) => &e.expression,
                Expression::TSNonNullExpression(e) => &e.expression,
                Expression::TSTypeAssertion(e) => &e.expression,
                Expression::StaticMemberExpression(m) => &m.object,
                Expression::ComputedMemberExpression(m) => &m.object,
                Expression::PrivateFieldExpression(m) => &m.object,
                Expression::Identifier(_) => return true,
                _ => return false,
            };
        }
    })
    // An expression that does not parse fails the file earlier.
    .unwrap_or(true)
}

/// `bind:group` on a snippet parameter.
fn bind_group_snippet_check(
    d: &svn_parser::ast::Directive,
    expression: Option<svn_core::Range>,
    ctx: &mut LintContext<'_>,
) {
    let base = match expression {
        Some(r) => ctx
            .source
            .get(r.start as usize..r.end as usize)
            .and_then(crate::scope_util::base_identifier_of_text),
        None => Some(d.name.to_string()),
    };
    let Some(base) = base else {
        return;
    };
    let Some(tree) = &ctx.scope_tree else {
        return;
    };
    let is_snippet_parameter = tree
        .resolve(tree.innermost_template_scope_at(d.range.start), &base)
        .is_some_and(|bid| tree.binding(bid).kind == crate::scope::BindingKind::Snippet);
    if is_snippet_parameter {
        ctx.emit_error(
            Code::bind_group_invalid_snippet_parameter,
            messages::bind_group_invalid_snippet_parameter(),
            d.range,
        );
    }
}

/// The parser's `parent_is_head`: the nearest `<svelte:head>`,
/// element or component frame is a `<svelte:head>`.
fn parent_is_head(path: &[PathFrame]) -> bool {
    for frame in path.iter().rev() {
        match frame {
            PathFrame::SvelteHead => return true,
            PathFrame::RegularElement { .. }
            | PathFrame::Component {
                kind: crate::walk::ComponentKind::Component,
                ..
            } => return false,
            _ => {}
        }
    }
    false
}

/// `TitleElement.js`: a `<title>` in `<svelte:head>` takes no
/// attributes and holds only text and `{expression}` tags.
fn visit_title_element(el: &Element, ctx: &mut LintContext<'_>) {
    if let Some(attr) = el
        .attributes
        .iter()
        .find(|a| !matches!(a, Attribute::Comment(_)))
    {
        ctx.emit_error(
            Code::title_illegal_attribute,
            messages::title_illegal_attribute(),
            attr.range(),
        );
    }
    for child in &el.children.nodes {
        let allowed = match child {
            svn_parser::ast::Node::Text(_) => true,
            svn_parser::ast::Node::Interpolation(i) => {
                i.kind == svn_parser::InterpolationKind::Expression
            }
            _ => false,
        };
        if !allowed {
            ctx.emit_error(
                Code::title_invalid_content,
                messages::title_invalid_content(),
                child.range(),
            );
        }
    }
}

/// `validate_slot_attribute` for a `<svelte:fragment>`'s `slot`
/// attribute.
pub(crate) fn validate_fragment_slot_attribute(attr: &Attribute, ctx: &mut LintContext<'_>) {
    if attribute_name(attr) != Some("slot") {
        return;
    }
    let text = match attr {
        Attribute::Plain(p) => static_text_value(p, ctx.source),
        _ => None,
    };
    validate_slot_attribute(ctx, text, attr.range(), false);
}

/// The name of a plain, expression or shorthand attribute.
pub(crate) fn plain_attribute_name(attr: &Attribute) -> Option<&str> {
    attribute_name(attr)
}
