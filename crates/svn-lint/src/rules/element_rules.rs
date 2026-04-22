//! Rules that fire on regular DOM elements.

use svn_parser::ast::{AttrValuePart, Attribute, DirectiveKind, Element};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::util::{is_mathml_element, is_svg_element, is_void_element};

/// Reference: html attribute name → correct JSX-like name (e.g.
/// `className` → `class`). Mirrors upstream
/// `phases/2-analyze/visitors/shared/element.js::react_attributes`.
const REACT_ATTRIBUTE_RENAMES: &[(&str, &str)] = &[("className", "class"), ("htmlFor", "for")];

/// Dispatch for regular DOM elements.
pub fn visit(
    el: &Element,
    ctx: &mut LintContext<'_>,
    parent_tag: Option<&str>,
    ancestors: &[String],
    inside_control_block: bool,
) {
    // node_invalid_placement_ssr: fires when the HTML5 tree-model
    // check says this child can't be here. Upstream emits the
    // warning variant only from inside a control-flow block; the
    // plain tree violation is an error elsewhere. We only handle
    // the warning.
    if inside_control_block {
        // Upstream semantics: the parent check and ancestor check
        // don't both fire for the same disallowed pair. Upstream's
        // `RegularElement.js` runs the parent check once, then —
        // walking OUT through further regular-element ancestors —
        // runs the ancestor check against an extending list. The
        // direct parent is NOT re-checked via the ancestor path.
        //
        // Here `ancestors` is outer-to-inner and includes the parent
        // as the last element. Strip it for the ancestor pass.
        let mut parent_check_fired = false;
        if let Some(parent) = parent_tag
            && let Some(msg) = crate::html5::is_tag_valid_with_parent(el.name.as_str(), parent)
        {
            let full = messages::node_invalid_placement_ssr(&msg);
            ctx.emit(Code::node_invalid_placement_ssr, full, el.range);
            parent_check_fired = true;
        }
        if ancestors.len() > 1 {
            // Walk from innermost outer ancestor to outermost,
            // extending the list each step. Fire on first match.
            let mut list: Vec<&str> = Vec::new();
            // Start with parent (upstream ancestors[0]).
            list.push(ancestors.last().unwrap().as_str());
            for outer in ancestors.iter().rev().skip(1) {
                list.push(outer.as_str());
                if parent_check_fired {
                    // Skip the first iteration — already handled by
                    // the parent check. Continue accumulating but
                    // don't refire on identical disallowed_children
                    // hits already reported.
                }
                if let Some(msg) = crate::html5::is_tag_valid_with_ancestor(el.name.as_str(), &list)
                {
                    let full = messages::node_invalid_placement_ssr(&msg);
                    ctx.emit(Code::node_invalid_placement_ssr, full, el.range);
                    break;
                }
            }
        }
        let _ = parent_check_fired;
    }

    // slot_element_deprecated: `<slot>` in runes mode (non-custom-element).
    // Upstream `visitors/SlotElement.js:14` passes the whole node.
    if ctx.runes && el.name == "slot" {
        let msg = messages::slot_element_deprecated();
        ctx.emit(Code::slot_element_deprecated, msg, el.range);
    }

    // component_name_lowercase: the tag starts lowercase AND resolves
    // to an import-kind binding with zero references in the script.
    // Upstream: `visitors/RegularElement.js:120-127`.
    if let Some(tree) = &ctx.scope_tree
        && let Some(bid) = tree.resolve_from_template(el.name.as_str())
    {
        let b = tree.binding(bid);
        if b.declaration_kind == crate::scope::DeclarationKind::Import
            && b.references.is_empty()
        {
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

    let is_custom = is_custom_element_name(&el.name);
    let parent = AttrParent::RegularElement { is_custom };

    // Per-attribute rules.
    for attr in &el.attributes {
        visit_attribute(attr, ctx, parent);
    }

    // a11y dispatch — runs after the basic attribute rules because
    // upstream's `check_element` path sits in the analyze visitors
    // and depends on the collected attribute map.
    crate::rules::a11y_rules::visit_regular(el, ctx, ancestors);
}

/// Custom element: tag name contains `-` and isn't known HTML.
/// Upstream: `phases/nodes.js::is_custom_element_node`.
fn is_custom_element_name(name: &str) -> bool {
    name.contains('-')
}

/// Parent kinds understood by the shared attribute visitor. Drives
/// rule dispatch (e.g. `attribute_quoted` only on Component/
/// SvelteComponent/SvelteSelf/custom-element;
/// `event_directive_deprecated` only on RegularElement/SvelteElement).
#[derive(Clone, Copy)]
pub(crate) enum AttrParent {
    /// A regular HTML element.
    RegularElement { is_custom: bool },
    /// A `<Component>` invocation.
    Component,
    /// `<svelte:component>` or `<svelte:self>`.
    SvelteComponentLike,
    /// `<svelte:element>` — dynamic element.
    SvelteElement,
    /// Any other `<svelte:*>` (options/window/head/body/document/fragment/boundary).
    OtherSvelte,
}

impl AttrParent {
    fn is_quotable(self) -> bool {
        matches!(
            self,
            Self::Component | Self::SvelteComponentLike | Self::RegularElement { is_custom: true }
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

pub(crate) fn visit_attribute(attr: &Attribute, ctx: &mut LintContext<'_>, parent: AttrParent) {
    let parent_is_quotable = parent.is_quotable();
    let fires_event_directive = parent.fires_event_directive_deprecated();
    let parent_is_regular_or_svelte =
        matches!(parent, AttrParent::RegularElement { .. } | AttrParent::SvelteElement);
    match attr {
        Attribute::Plain(p) => {
            let name = p.name.as_str();

            // attribute_illegal_colon — upstream:
            // `attr.name.includes(':')` AND NOT xmlns:/xlink:/xml:
            if name.contains(':')
                && !name.starts_with("xmlns:")
                && !name.starts_with("xlink:")
                && !name.starts_with("xml:")
            {
                let msg = messages::attribute_illegal_colon();
                ctx.emit(Code::attribute_illegal_colon, msg, p.range);
            }

            // attribute_avoid_is: `is="..."` on any element.
            if name == "is" {
                let msg = messages::attribute_avoid_is();
                ctx.emit(Code::attribute_avoid_is, msg, p.range);
            }

            // attribute_invalid_property_name: React-style name.
            if let Some(correct) = REACT_ATTRIBUTE_RENAMES
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

            // attribute_global_event_reference only fires on the
            // expression / shorthand forms below.
        }
        Attribute::Shorthand(s) => {
            // `{onkeydown}` — name IS the value identifier. Fire iff
            // the name starts with `on` and has no local binding in
            // the instance/module scope.
            let name = s.name.as_str();
            if parent_is_regular_or_svelte
                && name.starts_with("on")
                && name.len() > 2
                && let Some(tree) = &ctx.scope_tree
                && !tree.is_declared_anywhere(name)
            {
                let msg = messages::attribute_global_event_reference(name);
                ctx.emit(Code::attribute_global_event_reference, msg, s.range);
            }
        }
        Attribute::Expression(e) => {
            let name = e.name.as_str();
            if name.contains(':')
                && !name.starts_with("xmlns:")
                && !name.starts_with("xlink:")
                && !name.starts_with("xml:")
            {
                let msg = messages::attribute_illegal_colon();
                ctx.emit(Code::attribute_illegal_colon, msg, e.range);
            }
            if let Some(correct) = REACT_ATTRIBUTE_RENAMES
                .iter()
                .find_map(|(wrong, right)| if *wrong == name { Some(*right) } else { None })
            {
                let msg = messages::attribute_invalid_property_name(name, correct);
                ctx.emit(Code::attribute_invalid_property_name, msg, e.range);
            }
            // attribute_global_event_reference: `on{event}={ident}`
            // where `ident === attribute.name` and `ident` has no
            // local binding. Upstream: shared/element.js:62-75.
            if parent_is_regular_or_svelte
                && name.starts_with("on")
                && name.len() > 2
                && let Some(tree) = &ctx.scope_tree
            {
                let expr_src = ctx
                    .source
                    .get(
                        e.expression_range.start as usize..e.expression_range.end as usize,
                    )
                    .map(str::trim);
                if expr_src == Some(name) && !tree.is_declared_anywhere(name) {
                    let msg = messages::attribute_global_event_reference(name);
                    ctx.emit(Code::attribute_global_event_reference, msg, e.range);
                }
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
        }
        _ => {}
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
