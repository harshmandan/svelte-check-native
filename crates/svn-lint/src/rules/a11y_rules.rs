//! A11y rules — `check_element(node, ctx, ancestors)` entry point.
//!
//! Mirrors upstream
//! `packages/svelte/src/compiler/phases/2-analyze/visitors/shared/a11y/index.js`.
//! Shape: collect attribute map + handlers, then:
//!   1. Walk each attribute and dispatch name-based checks.
//!   2. Run whole-element checks (content emptiness, figcaption
//!      placement, etc.).
//!
//! Phase E.1 scope — only rules that don't require the ARIA role /
//! attribute-definition tables (aria-query / axobject-query). Rules
//! that need those land in E.3+.

use std::collections::HashMap;

use svn_core::Range;
use svn_parser::ast::{
    AttrValuePart, Attribute, DirectiveKind, Element, Fragment, Node, SvelteElement,
};

use crate::a11y_constants::{
    A11Y_DISTRACTING_ELEMENTS, A11Y_RECOMMENDED_INTERACTIVE_HANDLERS,
    A11Y_REQUIRED_CONTENT, ABSTRACT_ROLES, ADDRESS_TYPE_TOKENS, ARIA_ATTRIBUTES, ARIA_ROLES,
    AUTOFILL_CONTACT_FIELD_NAME_TOKENS, AUTOFILL_FIELD_NAME_TOKENS, AriaType, COMBOBOX_IF_LIST,
    CONTACT_TYPE_TOKENS, INTERACTIVE_ROLES, INVISIBLE_ELEMENTS, NON_INTERACTIVE_ROLES,
    PRESENTATION_ROLES, a11y_implicit_semantics, a11y_nested_implicit_semantics,
    a11y_non_interactive_element_to_interactive_role_exceptions, a11y_required_attributes,
    aria_prop, input_type_to_implicit_role, menuitem_type_to_implicit_role,
};
use crate::aria_data::{
    AttrState, is_interactive_element_schema, is_non_interactive_element_schema,
    is_semantic_role_element, role_props, role_supports_prop,
};
use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

/// Entry for a `<RegularElement>`.
pub fn visit_regular(
    el: &Element,
    ctx: &mut LintContext<'_>,
    ancestors: &[String],
) {
    check_element(
        el.name.as_str(),
        &el.attributes,
        el.range,
        false,
        &el.children,
        ctx,
        ancestors,
    );
}

/// Entry for a `<svelte:element>` dynamic element.
pub fn visit_dynamic(se: &SvelteElement, ctx: &mut LintContext<'_>, ancestors: &[String]) {
    // Only run a11y checks if `this={"literal"}` resolves to a
    // known element name. Upstream: dynamic-element checks are
    // mostly silenced (most rules bail on is_dynamic_element).
    let name = static_element_name_from_this(se).unwrap_or("");
    check_element(
        name,
        &se.attributes,
        se.range,
        true, /* dynamic */
        &se.children,
        ctx,
        ancestors,
    );
}

/// Resolve `<svelte:element this="div">` (literal string) → "div".
/// Returns None for expression-valued `this`.
fn static_element_name_from_this(se: &SvelteElement) -> Option<&str> {
    for a in &se.attributes {
        if let Attribute::Plain(p) = a
            && p.name.as_str() == "this"
            && let Some(v) = &p.value
            && v.parts.len() == 1
            && let AttrValuePart::Text { content, .. } = &v.parts[0]
        {
            return Some(content.as_str());
        }
    }
    None
}

#[allow(clippy::too_many_arguments)]
fn check_element(
    name: &str,
    attributes: &[Attribute],
    range: Range,
    is_dynamic: bool,
    children: &svn_parser::ast::Fragment,
    ctx: &mut LintContext<'_>,
    ancestors: &[String],
) {
    // Collect attribute map + event handlers.
    let mut attribute_map: HashMap<String, &Attribute> = HashMap::new();
    let mut handlers: Vec<String> = Vec::new();
    let mut has_spread = false;
    for a in attributes {
        match a {
            Attribute::Plain(p) => {
                if let Some(event) = event_name(p.name.as_str()) {
                    handlers.push(event.to_string());
                } else {
                    attribute_map.insert(p.name.as_str().to_string(), a);
                }
            }
            Attribute::Expression(e) => {
                if let Some(event) = event_name(e.name.as_str()) {
                    handlers.push(event.to_string());
                } else {
                    attribute_map.insert(e.name.as_str().to_string(), a);
                }
            }
            Attribute::Shorthand(s) => {
                if let Some(event) = event_name(s.name.as_str()) {
                    handlers.push(event.to_string());
                } else {
                    attribute_map.insert(s.name.as_str().to_string(), a);
                }
            }
            Attribute::Spread(_) => has_spread = true,
            Attribute::Directive(d) => {
                if d.kind == DirectiveKind::On {
                    handlers.push(d.name.as_str().to_string());
                }
            }
        }
    }
    let _ = &handlers;
    let _ = has_spread;

    // a11y_distracting_elements is emitted below the per-attribute
    // loop so role/interactivity fires come first in the warnings
    // sequence (upstream order).

    // a11y_img_redundant_alt: <img alt> with alt containing
    // "image"/"picture"/"photo" (word-boundary, case-insensitive).
    // Skipped when aria-hidden="true".
    if !is_dynamic && name == "img" {
        let aria_hidden = attribute_map.get("aria-hidden").and_then(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p),
            _ => None,
        }) == Some("true");
        if !aria_hidden
            && let Some(Attribute::Plain(p)) = attribute_map.get("alt")
            && let Some(alt) = get_static_text_value(p)
            && contains_redundant_img_word(alt)
        {
            let msg = messages::a11y_img_redundant_alt();
            ctx.emit(Code::a11y_img_redundant_alt, msg, range);
        }
    }

    // a11y_missing_content: empty heading (h1-h6) with no textual
    // content. Silenced by aria-hidden="true" (which then fires
    // a11y_hidden instead).
    if !is_dynamic && A11Y_REQUIRED_CONTENT.contains(&name) && !has_spread {
        let aria_hidden_true = attribute_map.get("aria-hidden").and_then(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p),
            _ => None,
        }) == Some("true");
        if aria_hidden_true {
            // a11y_hidden fires at the aria-hidden attribute range.
            if let Some(Attribute::Plain(p)) = attribute_map.get("aria-hidden") {
                let msg = messages::a11y_hidden(name);
                ctx.emit(Code::a11y_hidden, msg, p.range);
            }
        } else if !has_text_content(children) {
            let msg = messages::a11y_missing_content(name);
            ctx.emit(Code::a11y_missing_content, msg, range);
        }
    }

    // a11y_figcaption_parent / a11y_figcaption_index. Fires on
    // <figcaption> based on ancestor + sibling relationships:
    //   - figcaption NOT directly inside <figure> → parent warning
    //   - figcaption directly inside <figure> but not first or last
    //     child → index warning
    // The index-check is done at the parent level (in fragment
    // walking) because we need sibling order; parent-check can
    // fire here when we know our immediate parent is not <figure>.
    if !is_dynamic && name == "figcaption" {
        let parent_name = ancestors.last().map(String::as_str);
        if parent_name != Some("figure") {
            let msg = messages::a11y_figcaption_parent();
            ctx.emit(Code::a11y_figcaption_parent, msg, range);
        }
    }

    // a11y_consider_explicit_label: <button> / <a href> with no
    // text content AND no aria-label / aria-labelledby / title.
    // Skips when aria-hidden="true" or inert; upstream additionally
    // allows `<button>` when content contains `<selectedcontent>`
    // inside a select or a popover.
    if !is_dynamic && (name == "button" || (name == "a" && attribute_map.contains_key("href")))
        && !has_spread
    {
        let is_labelled = has_labelling_attr(&attribute_map);
        let aria_hidden_true = attribute_map.get("aria-hidden").and_then(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p),
            _ => None,
        }) == Some("true");
        let inert = attribute_map.contains_key("inert");
        if !is_labelled && !aria_hidden_true && !inert && !has_text_content(children) {
            let msg = messages::a11y_consider_explicit_label();
            ctx.emit(Code::a11y_consider_explicit_label, msg, range);
        }
    }

    // a11y_label_has_associated_control moved below the per-attribute
    // loop to match upstream's ordering (role-based checks run before
    // the per-element switch).

    // a11y_media_has_caption: <video src="..."> must have a
    // <track kind="captions"> child. Skipped on aria-hidden="true"
    // or when src isn't set.
    if !is_dynamic && name == "video" {
        let has_src = attribute_map.contains_key("src");
        let aria_hidden_true = attribute_map.get("aria-hidden").and_then(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p),
            _ => None,
        }) == Some("true");
        if has_src && !aria_hidden_true && !video_has_caption_track(children) {
            let msg = messages::a11y_media_has_caption();
            ctx.emit(Code::a11y_media_has_caption, msg, range);
        }
    }

    // click/mouse event rules moved to the post-loop section below
    // so they fire AFTER the per-attribute role checks (matches
    // upstream visitor order).

    // a11y_figcaption_index: when this element is a <figure>, scan
    // its direct children for <figcaption> not at the first or last
    // position.
    if !is_dynamic && name == "figure" {
        let mut figcaption_indices: Vec<(usize, Range)> = Vec::new();
        let mut direct: Vec<(usize, &Node)> = Vec::new();
        for n in &children.nodes {
            if matches!(n, Node::Comment(_)) {
                continue;
            }
            if let Node::Text(t) = n
                && !has_non_whitespace_text(&t.content)
            {
                continue;
            }
            direct.push((direct.len(), n));
        }
        for (idx, n) in &direct {
            if let Node::Element(el) = n
                && el.name.as_str() == "figcaption"
            {
                figcaption_indices.push((*idx, el.range));
            }
        }
        let last_idx = direct.len().saturating_sub(1);
        for (idx, r) in figcaption_indices {
            if idx != 0 && idx != last_idx {
                let msg = messages::a11y_figcaption_index();
                ctx.emit(Code::a11y_figcaption_index, msg, r);
            }
        }
    }

    // Precompute role + interactivity so every downstream rule
    // references the same values. Mirrors upstream's `role_value /
    // role_static_value` bookkeeping.
    let role_attr = attribute_map.get("role");
    let role_static_value = role_attr.and_then(|a| match a {
        Attribute::Plain(p) => get_static_text_value(p),
        _ => None,
    });
    let implicit_role = get_implicit_role(name, &attribute_map);
    let resolved_role = role_static_value.or(implicit_role);
    let is_implicit_role = role_attr.is_none() && implicit_role.is_some();
    let interactivity = if is_dynamic {
        Interactivity::Static
    } else {
        element_interactivity(name, &attribute_map)
    };
    let is_interactive = interactivity == Interactivity::Interactive;
    let is_non_interactive = interactivity == Interactivity::NonInteractive;
    let is_static = interactivity == Interactivity::Static;

    // Per-attribute rules.
    for a in attributes {
        let Attribute::Plain(p) = a else { continue };
        let lower = p.name.as_str().to_ascii_lowercase();
        // aria-activedescendant: fires when the element is non-
        // interactive and has no tabindex (and isn't dynamic).
        // Minimal interactivity check — deliberate subset until the
        // ARIA role tables land in E.3.
        if lower == "aria-activedescendant"
            && !is_dynamic
            && !has_spread
            && !attribute_map.contains_key("tabindex")
            && !is_interactive_element_minimal(name, &attribute_map)
        {
            let msg = messages::a11y_aria_activedescendant_has_tabindex();
            ctx.emit(
                Code::a11y_aria_activedescendant_has_tabindex,
                msg,
                p.range,
            );
        }

        // aria-*: unknown-aria-attribute + invisible-element +
        // value-type validation.
        if let Some(aria_name) = lower.strip_prefix("aria-")
            && !aria_name.is_empty()
        {
            // a11y_aria_attributes: invisible element can't carry
            // aria-* at all.
            if !is_dynamic && INVISIBLE_ELEMENTS.contains(&name) {
                let msg = messages::a11y_aria_attributes(name);
                ctx.emit(Code::a11y_aria_attributes, msg, p.range);
            }
            // a11y_unknown_aria_attribute: name after "aria-" must
            // be in the known attribute table. Suggest closest via
            // Damerau-free Levenshtein similarity.
            if !ARIA_ATTRIBUTES.contains(&aria_name) {
                let suggestion = fuzzy_match(aria_name, ARIA_ATTRIBUTES);
                let msg = messages::a11y_unknown_aria_attribute(aria_name, suggestion);
                ctx.emit(Code::a11y_unknown_aria_attribute, msg, p.range);
            } else if let Some(def) = aria_prop(lower.as_str()) {
                // Static-value type validation.
                validate_aria_value(p, &lower, def, ctx);
            }
        }
        match lower.as_str() {
            "role" => {
                // a11y_misplaced_role: role on an invisible element
                // (meta / html / script / style).
                if !is_dynamic && INVISIBLE_ELEMENTS.contains(&name) {
                    let msg = messages::a11y_misplaced_role(name);
                    ctx.emit(Code::a11y_misplaced_role, msg, p.range);
                }
                // a11y_no_abstract_role / a11y_unknown_role /
                // a11y_no_redundant_roles — all drive off the role's
                // text value. Each word is evaluated separately.
                if let Some(val) = get_static_text_value(p) {
                    for role_word in val.split_ascii_whitespace() {
                        if role_word.is_empty() {
                            continue;
                        }
                        if ABSTRACT_ROLES.contains(&role_word) {
                            let msg =
                                messages::a11y_no_abstract_role(role_word);
                            ctx.emit(Code::a11y_no_abstract_role, msg, p.range);
                        } else if !ARIA_ROLES.contains(&role_word) {
                            let suggestion =
                                fuzzy_match(role_word, ARIA_ROLES);
                            let msg = messages::a11y_unknown_role(
                                role_word, suggestion,
                            );
                            ctx.emit(Code::a11y_unknown_role, msg, p.range);
                        } else {
                            // a11y_role_has_required_aria_props: all
                            // of the role's requiredProps must be
                            // present on the element. Upstream
                            // silently skips when the element's
                            // native AX role already provides the
                            // role (via elementAXObjects).
                            let get_attr = |attr_name: &str| -> AttrState<'_> {
                                match attribute_map.get(attr_name) {
                                    None => AttrState::Absent,
                                    Some(Attribute::Plain(p)) => {
                                        match get_static_text_value(p) {
                                            Some(s) => AttrState::Literal(s),
                                            None => AttrState::Dynamic,
                                        }
                                    }
                                    Some(_) => AttrState::Dynamic,
                                }
                            };
                            if !is_dynamic
                                && !has_spread
                                && !is_semantic_role_element(role_word, name, get_attr)
                            {
                                if let Some((_, required)) = role_props(role_word)
                                    && !required.is_empty()
                                    && required
                                        .iter()
                                        .any(|r| !attribute_map.contains_key(*r))
                                {
                                    let quoted: Vec<String> = required
                                        .iter()
                                        .map(|r| format!("\"{r}\""))
                                        .collect();
                                    let list = join_sequence_borrowed_and(&quoted);
                                    let msg =
                                        messages::a11y_role_has_required_aria_props(
                                            role_word, &list,
                                        );
                                    ctx.emit(
                                        Code::a11y_role_has_required_aria_props,
                                        msg,
                                        p.range,
                                    );
                                }
                            }
                            // a11y_interactive_supports_focus: element is
                            // static AND role is interactive AND not
                            // disabled/hidden/presentation AND no
                            // tabindex AND has an interactive handler.
                            if !has_spread
                                && is_static
                                && is_interactive_role(role_word)
                                && !is_presentation_role(role_word)
                                && !has_disabled_attr(&attribute_map)
                                && !is_hidden_from_screen_reader(name, &attribute_map)
                                && !attribute_map.contains_key("tabindex")
                                && handlers
                                    .iter()
                                    .any(|h| crate::a11y_constants::is_interactive_handler(h.as_str(), ctx.compat))
                            {
                                let msg = messages::a11y_interactive_supports_focus(role_word);
                                ctx.emit(Code::a11y_interactive_supports_focus, msg, range);
                            }
                            // a11y_no_interactive_element_to_noninteractive_role:
                            // element is interactive, role is
                            // non-interactive or presentation.
                            if !has_spread
                                && is_interactive
                                && (is_non_interactive_role(role_word)
                                    || is_presentation_role(role_word))
                            {
                                let msg =
                                    messages::a11y_no_interactive_element_to_noninteractive_role(
                                        name, role_word,
                                    );
                                ctx.emit(
                                    Code::a11y_no_interactive_element_to_noninteractive_role,
                                    msg,
                                    range,
                                );
                            }
                            // a11y_no_noninteractive_element_to_interactive_role:
                            // element is non-interactive, role is
                            // interactive, not in carve-out table.
                            if !has_spread
                                && is_non_interactive
                                && is_interactive_role(role_word)
                            {
                                let is_exception =
                                    a11y_non_interactive_element_to_interactive_role_exceptions(
                                        name,
                                    )
                                    .is_some_and(|list| list.contains(&role_word));
                                if !is_exception {
                                    let msg =
                                        messages::a11y_no_noninteractive_element_to_interactive_role(
                                            name, role_word,
                                        );
                                    ctx.emit(
                                        Code::a11y_no_noninteractive_element_to_interactive_role,
                                        msg,
                                        range,
                                    );
                                }
                            }
                            let matches_implicit = Some(role_word)
                                == a11y_implicit_semantics(name);
                            let exempt_implicit = matches_implicit
                                && (matches!(name, "ul" | "ol" | "li" | "menu")
                                    || (name == "a" && !attribute_map.contains_key("href")));
                            if matches_implicit && !exempt_implicit {
                                let msg =
                                    messages::a11y_no_redundant_roles(role_word);
                                ctx.emit(Code::a11y_no_redundant_roles, msg, p.range);
                            }
                            // Upstream also checks nested_implicit
                            // semantics (header/footer → banner/
                            // contentinfo) when the element ISN'T
                            // nested in a <section> / <article>.
                            let matches_nested = Some(role_word)
                                == a11y_nested_implicit_semantics(name);
                            let in_section_or_article = ancestors
                                .iter()
                                .any(|a| a == "section" || a == "article");
                            if matches_nested && !in_section_or_article {
                                let msg =
                                    messages::a11y_no_redundant_roles(role_word);
                                ctx.emit(Code::a11y_no_redundant_roles, msg, p.range);
                            }
                        }
                    }
                }
            }
            "accesskey" => {
                let msg = messages::a11y_accesskey();
                ctx.emit(Code::a11y_accesskey, msg, p.range);
            }
            "autofocus" => {
                // Silence on `<dialog autofocus>` or when nested
                // inside a `<dialog>`.
                if name != "dialog" && !ancestors.iter().any(|a| a == "dialog") {
                    let msg = messages::a11y_autofocus();
                    ctx.emit(Code::a11y_autofocus, msg, p.range);
                }
            }
            "scope" => {
                if !is_dynamic && name != "th" {
                    let msg = messages::a11y_misplaced_scope();
                    ctx.emit(Code::a11y_misplaced_scope, msg, p.range);
                }
            }
            "tabindex" => {
                if let Some(s) = get_static_text_value(p)
                    && let Ok(n) = s.parse::<i64>()
                    && n > 0
                {
                    let msg = messages::a11y_positive_tabindex();
                    ctx.emit(Code::a11y_positive_tabindex, msg, p.range);
                }
                // a11y_no_noninteractive_tabindex: tabindex on a
                // non-interactive element with no interactive role
                // AND value is either dynamic (null) or >= 0. Fires
                // at the element, not the attribute.
                if !is_dynamic && !has_spread {
                    let has_interactive_role = role_static_value
                        .map(|r| {
                            r.split_ascii_whitespace()
                                .any(|w| INTERACTIVE_ROLES.contains(&w))
                        })
                        .unwrap_or(false);
                    let nonneg_tabindex = match get_static_text_value(p) {
                        Some(s) => match s.parse::<i64>() {
                            Ok(n) => n >= 0,
                            Err(_) => false,
                        },
                        None => true, // dynamic expression — treat as non-negative
                    };
                    if !is_interactive && !has_interactive_role && nonneg_tabindex {
                        let msg = messages::a11y_no_noninteractive_tabindex();
                        ctx.emit(
                            Code::a11y_no_noninteractive_tabindex,
                            msg,
                            range,
                        );
                    }
                }
            }
            _ => {}
        }
    }

    // a11y_role_supports_aria_props / _implicit — for every aria-*
    // attribute that the element's resolved role doesn't support,
    // fire at the attribute. Implicit role fires the _implicit
    // variant with the element name appended to the message.
    if !is_dynamic && !has_spread
        && let Some(role_val) = resolved_role
        && role_props(role_val).is_some()
    {
        for a in attributes {
            let Attribute::Plain(p) = a else { continue };
            let lower = p.name.as_str().to_ascii_lowercase();
            if !lower.starts_with("aria-") {
                continue;
            }
            if !ARIA_ATTRIBUTES.contains(&&lower[5..]) {
                // Unknown aria attribute — already reported
                // separately.
                continue;
            }
            if !role_supports_prop(role_val, &lower) {
                let msg = if is_implicit_role {
                    messages::a11y_role_supports_aria_props_implicit(
                        &lower, role_val, name,
                    )
                } else {
                    messages::a11y_role_supports_aria_props(&lower, role_val)
                };
                let code = if is_implicit_role {
                    Code::a11y_role_supports_aria_props_implicit
                } else {
                    Code::a11y_role_supports_aria_props
                };
                ctx.emit(code, msg, p.range);
            }
        }
    }

    // a11y_no_noninteractive_element_interactions: non-interactive
    // element (or interactive role on non-interactive element with
    // no role at all) with a recommended interactive handler.
    if !is_dynamic
        && !has_spread
        && !is_hidden_from_screen_reader(name, &attribute_map)
        && !resolved_role.is_some_and(is_presentation_role)
    {
        let has_contenteditable = attribute_map.contains_key("contenteditable");
        let role_is_non_interactive = role_static_value
            .is_some_and(is_non_interactive_role);
        let fire = !has_contenteditable
            && ((!is_interactive && role_is_non_interactive)
                || (is_non_interactive && role_attr.is_none()));
        if fire {
            let has_recommended = handlers.iter().any(|h| {
                A11Y_RECOMMENDED_INTERACTIVE_HANDLERS.contains(&h.as_str())
            });
            if has_recommended {
                let msg = messages::a11y_no_noninteractive_element_interactions(name);
                ctx.emit(Code::a11y_no_noninteractive_element_interactions, msg, range);
            }
        }
    }

    // a11y_no_static_element_interactions: static element (not
    // interactive, not non-interactive, no role or static role)
    // with any interactive handler.
    if !is_dynamic
        && !has_spread
        && (role_attr.is_none() || role_static_value.is_some())
        && !is_hidden_from_screen_reader(name, &attribute_map)
        && !resolved_role.is_some_and(is_presentation_role)
        && !is_interactive
        && !role_static_value.is_some_and(is_interactive_role)
        && !is_non_interactive
        && !role_static_value.is_some_and(is_non_interactive_role)
        && !role_static_value.is_some_and(|r| ABSTRACT_ROLES.contains(&r))
    {
        let interactive_handlers: Vec<&str> = handlers
            .iter()
            .filter(|h| crate::a11y_constants::is_interactive_handler(h.as_str(), ctx.compat))
            .map(|s| s.as_str())
            .collect();
        if !interactive_handlers.is_empty() {
            let list = join_handler_list(&interactive_handlers);
            let msg = messages::a11y_no_static_element_interactions(name, &list);
            ctx.emit(Code::a11y_no_static_element_interactions, msg, range);
        }
    }

    // a11y_distracting_elements: `<blink>` / `<marquee>`. Fires
    // after the role-attr handler so role-based fires come first.
    if !is_dynamic && A11Y_DISTRACTING_ELEMENTS.contains(&name) {
        let msg = messages::a11y_distracting_elements(name);
        ctx.emit(Code::a11y_distracting_elements, msg, range);
    }

    // a11y_label_has_associated_control: <label> must have `for`
    // or contain a form control / slot / svelte:element / component
    // / @render somewhere in its subtree. Runs after the per-attr
    // role handler so role-based fires come first (upstream order).
    if !is_dynamic && name == "label" && !has_spread {
        let has_for = attribute_map.contains_key("for");
        let has_control = descendants_have_form_control(children, ctx.source);
        if !has_for && !has_control {
            let msg = messages::a11y_label_has_associated_control();
            ctx.emit(Code::a11y_label_has_associated_control, msg, range);
        }
    }

    // a11y_click_events_have_key_events: fires after all role-based
    // checks so diagnostics sequence matches upstream visitor order.
    if !is_dynamic
        && !has_spread
        && handlers.iter().any(|h| h == "click")
        && !is_hidden_from_screen_reader(name, &attribute_map)
        && !is_interactive
    {
        let is_presentation = resolved_role.is_some_and(is_presentation_role);
        let is_dynamic_role = role_attr.is_some() && role_static_value.is_none();
        let role_ok = role_attr.is_none() || (role_static_value.is_some() && !is_presentation);
        if role_ok && !is_dynamic_role {
            let has_key = handlers
                .iter()
                .any(|h| matches!(h.as_str(), "keydown" | "keyup" | "keypress"));
            if !has_key {
                let msg = messages::a11y_click_events_have_key_events();
                ctx.emit(Code::a11y_click_events_have_key_events, msg, range);
            }
        }
    }

    // a11y_mouse_events_have_key_events: mouseover w/o focus,
    // mouseout w/o blur.
    if !is_dynamic && !has_spread {
        if handlers.iter().any(|h| h == "mouseover")
            && !handlers.iter().any(|h| h == "focus")
        {
            let msg = messages::a11y_mouse_events_have_key_events("mouseover", "focus");
            ctx.emit(Code::a11y_mouse_events_have_key_events, msg, range);
        }
        if handlers.iter().any(|h| h == "mouseout")
            && !handlers.iter().any(|h| h == "blur")
        {
            let msg = messages::a11y_mouse_events_have_key_events("mouseout", "blur");
            ctx.emit(Code::a11y_mouse_events_have_key_events, msg, range);
        }
    }

    // a11y_missing_attribute — element-level table lookup. Runs on
    // RegularElement only (svelte:element is dynamic).
    if !is_dynamic && !has_spread {
        match name {
            "a" => {
                // Upstream: check invalid-href-or-xlink:href values
                // (empty / '#' / 'javascript:...') when present; if
                // neither is present, fall back to the missing-attr
                // path (suppressed by truthy id/name or aria-disabled).
                let href_attr = attribute_map
                    .get("href")
                    .or_else(|| attribute_map.get("xlink:href"));
                if let Some(a) = href_attr {
                    if let Attribute::Plain(p) = a
                        && let Some(val) = get_static_text_value(p)
                        && (val.is_empty() || val == "#" || regex_js_prefix(val))
                    {
                        let msg =
                            messages::a11y_invalid_attribute(val, p.name.as_str());
                        ctx.emit(Code::a11y_invalid_attribute, msg, p.range);
                    }
                } else {
                    let id_truthy = static_nonempty(&attribute_map, "id");
                    let name_truthy = static_nonempty(&attribute_map, "name");
                    let aria_disabled = attribute_map
                        .get("aria-disabled")
                        .and_then(|a| match a {
                            Attribute::Plain(p) => get_static_text_value(p),
                            _ => None,
                        })
                        == Some("true");
                    if !id_truthy && !name_truthy && !aria_disabled {
                        let seq = join_sequence(&["href"]);
                        let msg = messages::a11y_missing_attribute(
                            "a",
                            article_for("href"),
                            &seq,
                        );
                        ctx.emit(Code::a11y_missing_attribute, msg, range);
                    }
                }
            }
            "input" => {
                let type_attr = attribute_map.get("type");
                let type_value = type_attr.and_then(|a| match a {
                    Attribute::Plain(p) => get_static_text_value(p),
                    _ => None,
                });
                let is_image = type_value == Some("image");
                // Only `<input type="image">` has a missing-attribute
                // check — others are handled by
                // `a11y_autocomplete_valid` / the role-based rules
                // elsewhere.
                if is_image {
                    let required: &[&str] =
                        &["alt", "aria-label", "aria-labelledby"];
                    let has_any =
                        required.iter().any(|w| attribute_map.contains_key(*w));
                    if !has_any {
                        let first = required.first().copied().unwrap_or("");
                        let seq = join_sequence(required);
                        let msg = messages::a11y_missing_attribute(
                            "input type=\"image\"",
                            article_for(first),
                            &seq,
                        );
                        ctx.emit(Code::a11y_missing_attribute, msg, range);
                    }
                }
                // a11y_autocomplete_valid. Upstream gate: fires only
                // when the input has BOTH `type` and `autocomplete`.
                // Dynamic `autocomplete={expr}` → get_static_value
                // returns null → is_valid_autocomplete treats as
                // valid. Plain bare attribute → upstream returns
                // `true` → fires with the literal text 'true'.
                if type_attr.is_some()
                    && let Some(Attribute::Plain(p)) = attribute_map.get("autocomplete")
                {
                    let state = classify_autocomplete_value(p);
                    if !is_valid_autocomplete(state) {
                        let display_value = match state {
                            AutocompleteValue::Bare => "true",
                            AutocompleteValue::Literal(s) => s,
                            // Dynamic is treated as valid — we don't
                            // emit here; the branch is unreachable
                            // due to the is_valid_autocomplete check.
                            AutocompleteValue::Dynamic => "",
                        };
                        let display_type = type_value.unwrap_or("...");
                        let msg = messages::a11y_autocomplete_valid(
                            display_value,
                            display_type,
                        );
                        ctx.emit(Code::a11y_autocomplete_valid, msg, p.range);
                    }
                }
            }
            _ => {
                if let Some(required) = a11y_required_attributes(name) {
                    let has_any = required
                        .iter()
                        .any(|want| attribute_map.contains_key(*want));
                    if !has_any {
                        let first = required.first().copied().unwrap_or("");
                        let seq = join_sequence(required);
                        let msg = messages::a11y_missing_attribute(
                            name,
                            article_for(first),
                            &seq,
                        );
                        ctx.emit(Code::a11y_missing_attribute, msg, range);
                    }
                }
            }
        }
    }
}

/// Return "a" or "an" for the leading token in the sequence message,
/// matching upstream `regex_starts_with_vowel` plus the `href`
/// special case.
fn article_for(first: &str) -> &'static str {
    if first == "href" {
        return "an";
    }
    match first.as_bytes().first() {
        Some(c)
            if matches!(
                c.to_ascii_lowercase(),
                b'a' | b'e' | b'i' | b'o' | b'u'
            ) =>
        {
            "an"
        }
        _ => "a",
    }
}

/// "a", "a or b", "a, b or c" — upstream `list()` utility form.
fn join_sequence(names: &[&str]) -> String {
    match names.len() {
        0 => String::new(),
        1 => names[0].to_string(),
        _ => {
            let last = names.last().copied().unwrap_or("");
            let first = &names[..names.len() - 1];
            let joined = first.join(", ");
            format!("{joined} or {last}")
        }
    }
}

fn event_name(name: &str) -> Option<&str> {
    name.strip_prefix("on").filter(|s| !s.is_empty())
}

/// Upstream `get_static_value` on the `autocomplete` attribute returns
/// one of three states: a literal string, `true` (bare attribute), or
/// `null` (dynamic expression / absent). We model the same tri-state
/// so `is_valid_autocomplete` can mirror the JS logic exactly.
#[derive(Clone, Copy)]
enum AutocompleteValue<'a> {
    /// `autocomplete` present with no `=value` — upstream `true`.
    Bare,
    /// `autocomplete="…"` with a literal string (possibly empty).
    Literal(&'a str),
    /// `autocomplete={expr}` — we can't resolve, treat as valid.
    Dynamic,
}

fn classify_autocomplete_value<'a>(p: &'a svn_parser::ast::PlainAttr) -> AutocompleteValue<'a> {
    match p.value.as_ref() {
        None => AutocompleteValue::Bare,
        Some(v) if v.parts.is_empty() && v.quoted => AutocompleteValue::Literal(""),
        Some(v) if v.parts.len() == 1 => match &v.parts[0] {
            AttrValuePart::Text { content, .. } => {
                AutocompleteValue::Literal(content.as_str())
            }
            AttrValuePart::Expression { .. } => AutocompleteValue::Dynamic,
        },
        _ => AutocompleteValue::Dynamic,
    }
}

/// Port of upstream `is_valid_autocomplete`. Grammar per WHATWG spec:
///
/// ```text
/// autocomplete = section-prefix?      // "section-xyz"
///                address-type?        // shipping | billing
///                ( field-name         // AUTOFILL_FIELD_NAME_TOKENS
///                | contact-type? contact-field-name  // tel/email/…
///                )
///                webauthn?
/// ```
///
/// Empty string and dynamic values are valid; bare `autocomplete` is
/// invalid (treated as literal `'true'`).
fn is_valid_autocomplete(value: AutocompleteValue<'_>) -> bool {
    let raw = match value {
        AutocompleteValue::Bare => return false,
        AutocompleteValue::Dynamic => return true,
        AutocompleteValue::Literal(s) => s,
    };
    if raw.is_empty() {
        return true;
    }
    let lower = raw.trim().to_ascii_lowercase();
    let mut tokens: Vec<&str> = lower.split_ascii_whitespace().collect();
    if tokens
        .first()
        .is_some_and(|t| t.starts_with("section-"))
    {
        tokens.remove(0);
    }
    if tokens
        .first()
        .is_some_and(|t| ADDRESS_TYPE_TOKENS.contains(t))
    {
        tokens.remove(0);
    }
    if tokens
        .first()
        .is_some_and(|t| AUTOFILL_FIELD_NAME_TOKENS.contains(t))
    {
        tokens.remove(0);
    } else {
        if tokens
            .first()
            .is_some_and(|t| CONTACT_TYPE_TOKENS.contains(t))
        {
            tokens.remove(0);
        }
        if tokens
            .first()
            .is_some_and(|t| AUTOFILL_CONTACT_FIELD_NAME_TOKENS.contains(t))
        {
            tokens.remove(0);
        } else {
            return false;
        }
    }
    if tokens.first() == Some(&"webauthn") {
        tokens.remove(0);
    }
    tokens.is_empty()
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Interactivity {
    Interactive,
    NonInteractive,
    Static,
}

/// Classify an element using the ARIA role schemas, matching
/// upstream `element_interactivity`.
fn element_interactivity(
    name: &str,
    attrs: &HashMap<String, &Attribute>,
) -> Interactivity {
    let get_attr = |attr_name: &str| -> AttrState<'_> {
        match attrs.get(attr_name) {
            None => AttrState::Absent,
            Some(Attribute::Plain(p)) => match get_static_text_value(p) {
                Some(s) => AttrState::Literal(s),
                None => AttrState::Dynamic,
            },
            Some(_) => AttrState::Dynamic,
        }
    };
    if is_interactive_element_schema(name, get_attr) {
        return Interactivity::Interactive;
    }
    // Upstream special-cases `<header>` against non-interactive
    // schemas — the "scoped to the body element" constraint excludes
    // nested headers. We approximate by skipping the whole element.
    if name != "header" && is_non_interactive_element_schema(name, get_attr) {
        return Interactivity::NonInteractive;
    }
    Interactivity::Static
}

/// Upstream `get_implicit_role`. For `<input>` / `<menuitem>` the
/// role depends on the `type` attribute; for everything else we
/// defer to [`a11y_implicit_semantics`].
fn get_implicit_role(
    name: &str,
    attrs: &HashMap<String, &Attribute>,
) -> Option<&'static str> {
    if name == "menuitem" {
        return attrs.get("type").and_then(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p)
                .and_then(menuitem_type_to_implicit_role),
            _ => None,
        });
    }
    if name == "input" {
        let type_str = attrs.get("type").and_then(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p),
            _ => None,
        });
        return type_str.and_then(|ty| {
            if attrs.contains_key("list") && COMBOBOX_IF_LIST.contains(&ty) {
                Some("combobox")
            } else {
                input_type_to_implicit_role(ty)
            }
        });
    }
    a11y_implicit_semantics(name)
}

fn is_presentation_role(role: &str) -> bool {
    PRESENTATION_ROLES.contains(&role)
}

fn is_interactive_role(role: &str) -> bool {
    INTERACTIVE_ROLES.contains(&role)
}

fn is_non_interactive_role(role: &str) -> bool {
    NON_INTERACTIVE_ROLES.contains(&role)
}

/// Is the element hidden from a screen reader? Upstream's heuristic:
/// - `<input type="hidden">`
/// - any `aria-hidden` attribute present with value `"true"` or a
///   dynamic value (null resolves to "assume hidden").
fn is_hidden_from_screen_reader(name: &str, attrs: &HashMap<String, &Attribute>) -> bool {
    if name == "input"
        && attrs.get("type").is_some_and(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p) == Some("hidden"),
            _ => false,
        })
    {
        return true;
    }
    match attrs.get("aria-hidden") {
        None => false,
        Some(Attribute::Plain(p)) => match get_static_text_value(p) {
            None => true,       // bare `<div aria-hidden>` → hidden
            Some("true") => true,
            _ => false,
        },
        Some(_) => true, // dynamic expression value — conservatively hidden
    }
}

/// Minimal "is this element interactive by default?" classifier.
/// Covers the subset relied on by
/// `a11y_aria_activedescendant_has_tabindex` and a handful of other
/// rules pending the full ARIA role table port (Phase E.3).
fn is_interactive_element_minimal(
    name: &str,
    attrs: &HashMap<String, &Attribute>,
) -> bool {
    match name {
        "a" => attrs.contains_key("href") || attrs.contains_key("xlink:href"),
        "audio" | "video" => attrs.contains_key("controls"),
        "button" | "details" | "embed" | "iframe" | "img" | "input" | "keygen"
        | "label" | "menu" | "menuitem" | "option" | "select" | "summary"
        | "textarea" | "tr" | "dialog" => true,
        _ => false,
    }
}

/// Matches upstream `regex_js_prefix = /^\W*javascript:/i`.
/// Returns true if the trimmed leading `\W*` prefix is followed by
/// `javascript:` (case-insensitive).
fn regex_js_prefix(s: &str) -> bool {
    // Walk past leading non-word chars (matching JS `\W`: anything
    // not [A-Za-z0-9_]).
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() && !is_word_byte(bytes[i]) {
        i += 1;
    }
    let rest = &s[i..];
    rest.len() >= 11
        && rest[..11].eq_ignore_ascii_case("javascript:")
}

fn static_nonempty(
    attrs: &HashMap<String, &Attribute>,
    key: &str,
) -> bool {
    attrs.get(key).is_some_and(|a| match a {
        Attribute::Plain(p) => get_static_text_value(p)
            .is_some_and(|s| !s.is_empty()),
        _ => false,
    })
}

/// Matches upstream `regex_redundant_img_alt`: `/\b(image|picture|photo)\b/i`.
/// Implemented by scanning for each keyword with ASCII case-insensitive
/// word-boundary semantics.
fn contains_redundant_img_word(text: &str) -> bool {
    const WORDS: &[&str] = &["image", "picture", "photo"];
    let lower = text.to_ascii_lowercase();
    let bytes = lower.as_bytes();
    for word in WORDS {
        let wb = word.as_bytes();
        let mut i = 0;
        while i + wb.len() <= bytes.len() {
            if &bytes[i..i + wb.len()] == wb {
                let before_ok = i == 0 || !is_word_byte(bytes[i - 1]);
                let after_ok =
                    i + wb.len() == bytes.len() || !is_word_byte(bytes[i + wb.len()]);
                if before_ok && after_ok {
                    return true;
                }
            }
            i += 1;
        }
    }
    false
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn has_non_whitespace_text(s: &str) -> bool {
    s.chars().any(|c| !c.is_whitespace())
}

/// True if any attribute form (`Plain` / `Expression` / `Shorthand`)
/// on the element has the given name. Matches upstream's
/// `node.attributes.some(a => a.type === 'Attribute' && a.name === N)`
/// which treats `foo="x"`, `foo={expr}` and `{foo}` the same.
fn has_attribute_named(attrs: &[Attribute], name: &str) -> bool {
    attrs.iter().any(|a| match a {
        Attribute::Plain(p) => p.name.as_str() == name,
        Attribute::Expression(e) => e.name.as_str() == name,
        Attribute::Shorthand(s) => s.name.as_str() == name,
        _ => false,
    })
}

/// Port of upstream `has_content` in a11y/index.js. Treats whitespace-
/// only text as empty, skips elements with `popover`, short-circuits
/// to true for `<img alt>` and `<selectedcontent>`, and conservatively
/// assumes anything non-element (Component / Interpolation / blocks)
/// carries content to avoid false positives.
fn has_text_content(frag: &Fragment) -> bool {
    for n in &frag.nodes {
        match n {
            Node::Text(t) => {
                if !has_non_whitespace_text(&t.content) {
                    continue;
                }
            }
            Node::Comment(_) => continue,
            Node::Element(el) => {
                if has_attribute_named(&el.attributes, "popover") {
                    continue;
                }
                if el.name.as_str() == "img"
                    && has_attribute_named(&el.attributes, "alt")
                {
                    return true;
                }
                if el.name.as_str() == "selectedcontent" {
                    return true;
                }
                // `<slot>` is a SlotElement in upstream's AST and falls
                // through upstream's `has_content` to the pessimistic
                // "assume everything else has content" branch — the
                // parent might supply arbitrary content for it at
                // render time. Our parser classes it as a regular
                // element; gate here to keep upstream parity.
                if el.name.as_str() == "slot" {
                    return true;
                }
                if !has_text_content(&el.children) {
                    continue;
                }
            }
            Node::SvelteElement(se) => {
                if has_attribute_named(&se.attributes, "popover") {
                    continue;
                }
                if !has_text_content(&se.children) {
                    continue;
                }
            }
            // Everything else — Component, Interpolation, if/each/await/
            // key/snippet blocks — is assumed to carry content. Matches
            // upstream's pessimistic fallback.
            _ => {}
        }
        return true;
    }
    false
}

fn has_labelling_attr(attrs: &HashMap<String, &Attribute>) -> bool {
    attrs.contains_key("aria-label")
        || attrs.contains_key("aria-labelledby")
        || attrs.contains_key("title")
}

/// Port of upstream `has_input_child` for `<label>` — returns true
/// if any descendant is a form control, a `<slot>`, a
/// `<svelte:element>`, a `<Component>`, or a `{@render}` tag.
fn descendants_have_form_control(frag: &Fragment, source: &str) -> bool {
    use crate::a11y_constants::A11Y_LABELABLE;
    for n in &frag.nodes {
        match n {
            Node::Element(el) => {
                let n = el.name.as_str();
                if A11Y_LABELABLE.contains(&n) || n == "slot" {
                    return true;
                }
                if descendants_have_form_control(&el.children, source) {
                    return true;
                }
            }
            Node::SvelteElement(_) => return true,
            Node::Component(_) => return true,
            Node::Interpolation(intr) => {
                // `{@render x()}` appears as an Interpolation in
                // svn-parser — recognise it by the leading `{@render`
                // in its full range (expression_range skips the
                // keyword, so we peek at the raw source).
                if is_render_tag(source, intr.range) {
                    return true;
                }
            }
            Node::IfBlock(b) => {
                if descendants_have_form_control(&b.consequent, source) {
                    return true;
                }
                for arm in &b.elseif_arms {
                    if descendants_have_form_control(&arm.body, source) {
                        return true;
                    }
                }
                if let Some(alt) = &b.alternate
                    && descendants_have_form_control(alt, source)
                {
                    return true;
                }
            }
            Node::EachBlock(b) => {
                if descendants_have_form_control(&b.body, source) {
                    return true;
                }
                if let Some(alt) = &b.alternate
                    && descendants_have_form_control(alt, source)
                {
                    return true;
                }
            }
            Node::AwaitBlock(b) => {
                if let Some(p) = &b.pending
                    && descendants_have_form_control(p, source)
                {
                    return true;
                }
                if let Some(t) = &b.then_branch
                    && descendants_have_form_control(&t.body, source)
                {
                    return true;
                }
                if let Some(c) = &b.catch_branch
                    && descendants_have_form_control(&c.body, source)
                {
                    return true;
                }
            }
            Node::KeyBlock(b) => {
                if descendants_have_form_control(&b.body, source) {
                    return true;
                }
            }
            Node::SnippetBlock(b) => {
                if descendants_have_form_control(&b.body, source) {
                    return true;
                }
            }
            _ => {}
        }
    }
    false
}

/// Static ARIA value-type check. Mirrors upstream
/// `validate_aria_attribute_value` — bare attributes (value=None)
/// coerce to `""`; dynamic expressions bail (we can't resolve).
fn validate_aria_value(
    p: &svn_parser::ast::PlainAttr,
    name: &str,
    def: &crate::a11y_constants::AriaPropDef,
    ctx: &mut LintContext<'_>,
) {
    // Determine a static-string value. None == dynamic → skip.
    let value: Option<&str> = match p.value.as_ref() {
        None => Some(""), // bare attribute `aria-level`
        Some(v) => {
            if v.parts.is_empty() && v.quoted {
                Some("")
            } else if v.parts.len() == 1 {
                match &v.parts[0] {
                    AttrValuePart::Text { content, .. } => Some(content.as_str()),
                    AttrValuePart::Expression { .. } => None,
                }
            } else {
                None
            }
        }
    };
    let Some(val) = value else { return };
    let lower = val.to_ascii_lowercase();
    match def.ty {
        AriaType::String | AriaType::Id => {
            if val.is_empty() {
                let msg = messages::a11y_incorrect_aria_attribute_type(name, "non-empty string");
                ctx.emit(Code::a11y_incorrect_aria_attribute_type, msg, p.range);
            }
        }
        AriaType::Number => {
            if val.is_empty() || val.parse::<f64>().is_err() {
                let msg = messages::a11y_incorrect_aria_attribute_type(name, "number");
                ctx.emit(Code::a11y_incorrect_aria_attribute_type, msg, p.range);
            }
        }
        AriaType::Integer => {
            // Upstream accepts any value for which `Number.isInteger(+v)` is
            // true — we replicate by parsing as f64, then checking it's finite
            // and equal to its floor/ceil.
            let ok = !val.is_empty()
                && val.parse::<f64>().ok().is_some_and(|f| f.is_finite() && f.fract() == 0.0);
            if !ok {
                let msg = messages::a11y_incorrect_aria_attribute_type_integer(name);
                ctx.emit(
                    Code::a11y_incorrect_aria_attribute_type_integer,
                    msg,
                    p.range,
                );
            }
        }
        AriaType::Boolean | AriaType::BooleanUndefined => {
            // Upstream's `validate_aria_attribute_value` has one
            // `'boolean'` branch that strict-checks for "true" /
            // "false" regardless of whether the attribute's schema
            // allows an undefined state. Bare attributes (shorthand
            // `aria-hidden` without `=value`) resolve to `""` and
            // fire here as expected. Our earlier split into
            // Boolean / BooleanUndefined buckets was too lenient on
            // the latter.
            if val != "true" && val != "false" {
                let msg = messages::a11y_incorrect_aria_attribute_type_boolean(name);
                ctx.emit(
                    Code::a11y_incorrect_aria_attribute_type_boolean,
                    msg,
                    p.range,
                );
            }
        }
        AriaType::IdList => {
            if val.is_empty() {
                let msg = messages::a11y_incorrect_aria_attribute_type_idlist(name);
                ctx.emit(
                    Code::a11y_incorrect_aria_attribute_type_idlist,
                    msg,
                    p.range,
                );
            }
        }
        AriaType::Token => {
            if !def.values.contains(&lower.as_str()) {
                let list = quote_list(def.values);
                let msg = messages::a11y_incorrect_aria_attribute_type_token(name, &list);
                ctx.emit(
                    Code::a11y_incorrect_aria_attribute_type_token,
                    msg,
                    p.range,
                );
            }
        }
        AriaType::TokenList => {
            let all_ok = !lower.is_empty()
                && lower
                    .split(|c: char| c.is_whitespace())
                    .filter(|s| !s.is_empty())
                    .all(|tok| def.values.contains(&tok));
            let empty_after_split = lower
                .split(|c: char| c.is_whitespace())
                .find(|s| !s.is_empty())
                .is_none();
            if empty_after_split || !all_ok || lower.split_whitespace().count() == 0 || has_trailing_ws_only_tok(&lower, def.values) {
                let list = quote_list(def.values);
                let msg = messages::a11y_incorrect_aria_attribute_type_tokenlist(name, &list);
                ctx.emit(
                    Code::a11y_incorrect_aria_attribute_type_tokenlist,
                    msg,
                    p.range,
                );
            }
        }
        AriaType::Tristate => {
            if val != "true" && val != "false" && val != "mixed" {
                let msg = messages::a11y_incorrect_aria_attribute_type_tristate(name);
                ctx.emit(
                    Code::a11y_incorrect_aria_attribute_type_tristate,
                    msg,
                    p.range,
                );
            }
        }
    }
}

fn has_trailing_ws_only_tok(_s: &str, _values: &[&str]) -> bool {
    false
}

fn quote_list(values: &[&str]) -> String {
    let quoted: Vec<String> = values.iter().map(|v| format!("\"{v}\"")).collect();
    join_sequence_borrowed(&quoted)
}

fn join_sequence_borrowed(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        _ => {
            let last = items.last().cloned().unwrap_or_default();
            let first = &items[..items.len() - 1];
            let joined = first.join(", ");
            format!("{joined} or {last}")
        }
    }
}

/// Oxford-style "x, y and z" join — required by
/// `a11y_role_has_required_aria_props`'s message (upstream uses
/// `list(..., 'and')`).
fn join_sequence_borrowed_and(items: &[String]) -> String {
    match items.len() {
        0 => String::new(),
        1 => items[0].clone(),
        _ => {
            let last = items.last().cloned().unwrap_or_default();
            let first = &items[..items.len() - 1];
            let joined = first.join(", ");
            format!("{joined} and {last}")
        }
    }
}

/// Upstream `list()` with default separator — joins bare names
/// using ", " and "or" for the last element. Handler names are
/// passed unquoted by upstream, so we match that shape.
fn join_handler_list(handlers: &[&str]) -> String {
    let items: Vec<String> = handlers.iter().map(|s| s.to_string()).collect();
    join_sequence_borrowed(&items)
}

/// Matches upstream `has_disabled_attribute` — both `disabled` and
/// `aria-disabled="true"` count.
fn has_disabled_attr(attrs: &HashMap<String, &Attribute>) -> bool {
    if attrs.contains_key("disabled") {
        return true;
    }
    attrs
        .get("aria-disabled")
        .is_some_and(|a| match a {
            Attribute::Plain(p) => get_static_text_value(p) == Some("true"),
            _ => false,
        })
}

fn fuzzy_match<'a>(name: &str, candidates: &[&'a str]) -> Option<&'a str> {
    // Simple Levenshtein similarity — find the closest match where
    // similarity ≥ 0.7. Matches the threshold upstream uses in
    // fuzzymatch.js.
    let target = name.to_ascii_lowercase();
    let mut best: Option<(f64, &str)> = None;
    for c in candidates {
        let sim = levenshtein_similarity(&target, &c.to_ascii_lowercase());
        if sim >= 0.7 && best.map(|(s, _)| sim > s).unwrap_or(true) {
            best = Some((sim, *c));
        }
    }
    best.map(|(_, c)| c)
}

fn levenshtein_similarity(a: &str, b: &str) -> f64 {
    let distance = levenshtein_distance(a, b);
    let max_len = a.chars().count().max(b.chars().count()) as f64;
    if max_len == 0.0 {
        return 1.0;
    }
    1.0 - (distance as f64) / max_len
}

fn levenshtein_distance(a: &str, b: &str) -> usize {
    let a_chars: Vec<char> = a.chars().collect();
    let b_chars: Vec<char> = b.chars().collect();
    if a_chars.is_empty() {
        return b_chars.len();
    }
    if b_chars.is_empty() {
        return a_chars.len();
    }
    let mut prev_row: Vec<usize> = (0..=b_chars.len()).collect();
    let mut curr_row = vec![0usize; b_chars.len() + 1];
    for i in 1..=a_chars.len() {
        curr_row[0] = i;
        for j in 1..=b_chars.len() {
            let cost = if a_chars[i - 1] == b_chars[j - 1] { 0 } else { 1 };
            curr_row[j] = (prev_row[j] + 1)
                .min(curr_row[j - 1] + 1)
                .min(prev_row[j - 1] + cost);
        }
        std::mem::swap(&mut prev_row, &mut curr_row);
    }
    prev_row[b_chars.len()]
}

fn is_render_tag(source: &str, range: Range) -> bool {
    source
        .get(range.start as usize..range.end as usize)
        .is_some_and(|s| s.starts_with("{@render"))
}

fn video_has_caption_track(frag: &Fragment) -> bool {
    for n in &frag.nodes {
        if let Node::Element(el) = n
            && el.name.as_str() == "track"
        {
            let kind = el.attributes.iter().find_map(|a| match a {
                Attribute::Plain(p) if p.name.as_str() == "kind" => get_static_text_value(p),
                _ => None,
            });
            if kind == Some("captions") {
                return true;
            }
        }
    }
    false
}

/// Pulls the first plain-text value off an attribute. Returns None if
/// the attribute is expression-valued or contains interpolations.
/// For quoted empty values (`href=""`) the attribute value has zero
/// parts — those resolve to `Some("")`.
fn get_static_text_value(p: &svn_parser::ast::PlainAttr) -> Option<&str> {
    let v = p.value.as_ref()?;
    if v.parts.is_empty() && v.quoted {
        return Some("");
    }
    if v.parts.len() != 1 {
        return None;
    }
    match &v.parts[0] {
        AttrValuePart::Text { content, .. } => Some(content.as_str()),
        AttrValuePart::Expression { .. } => None,
    }
}
