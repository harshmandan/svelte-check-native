//! Rules that fire on regular DOM elements.

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
                | PathFrame::SvelteElement
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
        visit_attribute(attr, ctx, parent);
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
    let path = &ctx.template_path;
    // The node that owns the fragment the element sits in.
    let direct_child_of = path.last();
    // The nearest component-like or custom-element ancestor.
    let owner = path.iter().rposition(|f| {
        matches!(
            f,
            PathFrame::Component { .. }
                | PathFrame::SvelteElement
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
            Some((_, PathFrame::Component { .. })) => {
                invalid_value.then_some(SlotError::InvalidValue)
            }
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
        None => {}
    }
}

enum SlotError {
    InvalidValue,
    InvalidPlacement,
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
    /// Any other `<svelte:*>` (options/head/fragment/boundary).
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

pub(crate) fn visit_attribute(attr: &Attribute, ctx: &mut LintContext<'_>, parent: AttrParent<'_>) {
    let parent_is_quotable = parent.is_quotable();
    let fires_event_directive = parent.fires_event_directive_deprecated();
    let parent_is_regular_or_svelte = matches!(
        parent,
        AttrParent::RegularElement { .. } | AttrParent::SvelteElement
    );
    let fires_invalid_property_name = parent_is_regular_or_svelte;
    let fires_attr_name_checks = !matches!(parent, AttrParent::OtherSvelte);
    // `validate_slot_attribute` runs for elements and components; for a
    // component a misplaced `slot` is not an error.
    let slot_owner_kind = match parent {
        AttrParent::RegularElement { .. } | AttrParent::SvelteElement => Some(false),
        AttrParent::Component | AttrParent::SvelteComponentLike => Some(true),
        _ => None,
    };
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

            if parent_is_regular_or_svelte && is_illegal_attribute_name(name) {
                ctx.emit_error(
                    Code::attribute_invalid_name,
                    messages::attribute_invalid_name(name),
                    p.range,
                );
            }

            // An `on*` attribute on an element must hold exactly one
            // expression; a quoted `"{handler}"` is that expression.
            if parent_is_regular_or_svelte && name.starts_with("on") && name.len() > 2 {
                match p.value.as_ref().map(|v| v.parts.as_slice()) {
                    Some(
                        [
                            AttrValuePart::Expression {
                                expression_range, ..
                            },
                        ],
                    ) => global_event_reference(name, *expression_range, p.range, ctx),
                    _ => ctx.emit_error(
                        Code::attribute_invalid_event_handler,
                        messages::attribute_invalid_event_handler(),
                        p.range,
                    ),
                }
            }

            if name == "slot"
                && let Some(is_component) = slot_owner_kind
            {
                let text = static_text_value(p, ctx.source);
                validate_slot_attribute(ctx, text, p.range, is_component);
            }
        }
        Attribute::Shorthand(s) => {
            let name = s.name.as_str();
            if name == "slot"
                && let Some(is_component) = slot_owner_kind
            {
                validate_slot_attribute(ctx, None, s.range, is_component);
            }
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
            if parent_is_regular_or_svelte && is_illegal_attribute_name(name) {
                ctx.emit_error(
                    Code::attribute_invalid_name,
                    messages::attribute_invalid_name(name),
                    e.range,
                );
            }
            if parent_is_regular_or_svelte && name.starts_with("on") && name.len() > 2 {
                global_event_reference(name, e.expression_range, e.range, ctx);
            }
            if name == "slot"
                && let Some(is_component) = slot_owner_kind
            {
                validate_slot_attribute(ctx, None, e.range, is_component);
            }
        }
        Attribute::Directive(d) => {
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
                }
                crate::rules::binding_rules::flush_template_write_violations(
                    ctx,
                    d.range.start,
                    true,
                );
                if d.name != "this" {
                    bind_value_check(d, ctx);
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
