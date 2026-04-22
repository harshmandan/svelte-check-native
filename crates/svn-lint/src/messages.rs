// GENERATED — do not edit. Run `cargo run -p xtask --bin regen-lint-catalog`.
//
// Source: .svelte-upstream/svelte/packages/svelte/messages/compile-warnings/*.md

#![allow(
    non_snake_case,
    dead_code,
    clippy::too_many_arguments,
    clippy::useless_format
)]

//! Message-text builders for each warning code.

/// Avoid using accesskey
pub fn a11y_accesskey() -> String {
    format!("Avoid using accesskey\nhttps://svelte.dev/e/a11y_accesskey")
}

/// An element with an aria-activedescendant attribute should have a tabindex value
pub fn a11y_aria_activedescendant_has_tabindex() -> String {
    format!(
        "An element with an aria-activedescendant attribute should have a tabindex value\nhttps://svelte.dev/e/a11y_aria_activedescendant_has_tabindex"
    )
}

/// `<%name%>` should not have aria-* attributes
pub fn a11y_aria_attributes(name: &str) -> String {
    format!(
        "`<{name}>` should not have aria-* attributes\nhttps://svelte.dev/e/a11y_aria_attributes",
        name = name
    )
}

/// '%value%' is an invalid value for 'autocomplete' on `<input type="%type%">`
pub fn a11y_autocomplete_valid(value: &str, type_: &str) -> String {
    format!(
        "'{value}' is an invalid value for 'autocomplete' on `<input type=\"{type_}\">`\nhttps://svelte.dev/e/a11y_autocomplete_valid",
        value = value,
        type_ = type_
    )
}

/// Avoid using autofocus
pub fn a11y_autofocus() -> String {
    format!("Avoid using autofocus\nhttps://svelte.dev/e/a11y_autofocus")
}

/// Visible, non-interactive elements with a click event must be accompanied by a keyboard event handler. Consider whether an interactive element such as `<button type="button">` or `<a>` might be more appropriate
pub fn a11y_click_events_have_key_events() -> String {
    format!(
        "Visible, non-interactive elements with a click event must be accompanied by a keyboard event handler. Consider whether an interactive element such as `<button type=\"button\">` or `<a>` might be more appropriate\nhttps://svelte.dev/e/a11y_click_events_have_key_events"
    )
}

/// Buttons and links should either contain text or have an `aria-label`, `aria-labelledby` or `title` attribute
pub fn a11y_consider_explicit_label() -> String {
    format!(
        "Buttons and links should either contain text or have an `aria-label`, `aria-labelledby` or `title` attribute\nhttps://svelte.dev/e/a11y_consider_explicit_label"
    )
}

/// Avoid `<%name%>` elements
pub fn a11y_distracting_elements(name: &str) -> String {
    format!(
        "Avoid `<{name}>` elements\nhttps://svelte.dev/e/a11y_distracting_elements",
        name = name
    )
}

/// `<figcaption>` must be first or last child of `<figure>`
pub fn a11y_figcaption_index() -> String {
    format!(
        "`<figcaption>` must be first or last child of `<figure>`\nhttps://svelte.dev/e/a11y_figcaption_index"
    )
}

/// `<figcaption>` must be an immediate child of `<figure>`
pub fn a11y_figcaption_parent() -> String {
    format!(
        "`<figcaption>` must be an immediate child of `<figure>`\nhttps://svelte.dev/e/a11y_figcaption_parent"
    )
}

/// `<%name%>` element should not be hidden
pub fn a11y_hidden(name: &str) -> String {
    format!(
        "`<{name}>` element should not be hidden\nhttps://svelte.dev/e/a11y_hidden",
        name = name
    )
}

/// Screenreaders already announce `<img>` elements as an image
pub fn a11y_img_redundant_alt() -> String {
    format!(
        "Screenreaders already announce `<img>` elements as an image\nhttps://svelte.dev/e/a11y_img_redundant_alt"
    )
}

/// The value of '%attribute%' must be a %type%
pub fn a11y_incorrect_aria_attribute_type(attribute: &str, type_: &str) -> String {
    format!(
        "The value of '{attribute}' must be a {type_}\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type",
        attribute = attribute,
        type_ = type_
    )
}

/// The value of '%attribute%' must be either 'true' or 'false'. It cannot be empty
pub fn a11y_incorrect_aria_attribute_type_boolean(attribute: &str) -> String {
    format!(
        "The value of '{attribute}' must be either 'true' or 'false'. It cannot be empty\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_boolean",
        attribute = attribute
    )
}

/// The value of '%attribute%' must be a string that represents a DOM element ID
pub fn a11y_incorrect_aria_attribute_type_id(attribute: &str) -> String {
    format!(
        "The value of '{attribute}' must be a string that represents a DOM element ID\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_id",
        attribute = attribute
    )
}

/// The value of '%attribute%' must be a space-separated list of strings that represent DOM element IDs
pub fn a11y_incorrect_aria_attribute_type_idlist(attribute: &str) -> String {
    format!(
        "The value of '{attribute}' must be a space-separated list of strings that represent DOM element IDs\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_idlist",
        attribute = attribute
    )
}

/// The value of '%attribute%' must be an integer
pub fn a11y_incorrect_aria_attribute_type_integer(attribute: &str) -> String {
    format!(
        "The value of '{attribute}' must be an integer\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_integer",
        attribute = attribute
    )
}

/// The value of '%attribute%' must be exactly one of %values%
pub fn a11y_incorrect_aria_attribute_type_token(attribute: &str, values: &str) -> String {
    format!(
        "The value of '{attribute}' must be exactly one of {values}\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_token",
        attribute = attribute,
        values = values
    )
}

/// The value of '%attribute%' must be a space-separated list of one or more of %values%
pub fn a11y_incorrect_aria_attribute_type_tokenlist(attribute: &str, values: &str) -> String {
    format!(
        "The value of '{attribute}' must be a space-separated list of one or more of {values}\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_tokenlist",
        attribute = attribute,
        values = values
    )
}

/// The value of '%attribute%' must be exactly one of true, false, or mixed
pub fn a11y_incorrect_aria_attribute_type_tristate(attribute: &str) -> String {
    format!(
        "The value of '{attribute}' must be exactly one of true, false, or mixed\nhttps://svelte.dev/e/a11y_incorrect_aria_attribute_type_tristate",
        attribute = attribute
    )
}

/// Elements with the '%role%' interactive role must have a tabindex value
pub fn a11y_interactive_supports_focus(role: &str) -> String {
    format!(
        "Elements with the '{role}' interactive role must have a tabindex value\nhttps://svelte.dev/e/a11y_interactive_supports_focus",
        role = role
    )
}

/// '%href_value%' is not a valid %href_attribute% attribute
pub fn a11y_invalid_attribute(href_value: &str, href_attribute: &str) -> String {
    format!(
        "'{href_value}' is not a valid {href_attribute} attribute\nhttps://svelte.dev/e/a11y_invalid_attribute",
        href_value = href_value,
        href_attribute = href_attribute
    )
}

/// A form label must be associated with a control
pub fn a11y_label_has_associated_control() -> String {
    format!(
        "A form label must be associated with a control\nhttps://svelte.dev/e/a11y_label_has_associated_control"
    )
}

/// `<video>` elements must have a `<track kind="captions">`
pub fn a11y_media_has_caption() -> String {
    format!(
        "`<video>` elements must have a `<track kind=\"captions\">`\nhttps://svelte.dev/e/a11y_media_has_caption"
    )
}

/// `<%name%>` should not have role attribute
pub fn a11y_misplaced_role(name: &str) -> String {
    format!(
        "`<{name}>` should not have role attribute\nhttps://svelte.dev/e/a11y_misplaced_role",
        name = name
    )
}

/// The scope attribute should only be used with `<th>` elements
pub fn a11y_misplaced_scope() -> String {
    format!(
        "The scope attribute should only be used with `<th>` elements\nhttps://svelte.dev/e/a11y_misplaced_scope"
    )
}

/// `<%name%>` element should have %article% %sequence% attribute
pub fn a11y_missing_attribute(name: &str, article: &str, sequence: &str) -> String {
    format!(
        "`<{name}>` element should have {article} {sequence} attribute\nhttps://svelte.dev/e/a11y_missing_attribute",
        name = name,
        article = article,
        sequence = sequence
    )
}

/// `<%name%>` element should contain text
pub fn a11y_missing_content(name: &str) -> String {
    format!(
        "`<{name}>` element should contain text\nhttps://svelte.dev/e/a11y_missing_content",
        name = name
    )
}

/// '%event%' event must be accompanied by '%accompanied_by%' event
pub fn a11y_mouse_events_have_key_events(event: &str, accompanied_by: &str) -> String {
    format!(
        "'{event}' event must be accompanied by '{accompanied_by}' event\nhttps://svelte.dev/e/a11y_mouse_events_have_key_events",
        event = event,
        accompanied_by = accompanied_by
    )
}

/// Abstract role '%role%' is forbidden
pub fn a11y_no_abstract_role(role: &str) -> String {
    format!(
        "Abstract role '{role}' is forbidden\nhttps://svelte.dev/e/a11y_no_abstract_role",
        role = role
    )
}

/// `<%element%>` cannot have role '%role%'
pub fn a11y_no_interactive_element_to_noninteractive_role(element: &str, role: &str) -> String {
    format!(
        "`<{element}>` cannot have role '{role}'\nhttps://svelte.dev/e/a11y_no_interactive_element_to_noninteractive_role",
        element = element,
        role = role
    )
}

/// Non-interactive element `<%element%>` should not be assigned mouse or keyboard event listeners
pub fn a11y_no_noninteractive_element_interactions(element: &str) -> String {
    format!(
        "Non-interactive element `<{element}>` should not be assigned mouse or keyboard event listeners\nhttps://svelte.dev/e/a11y_no_noninteractive_element_interactions",
        element = element
    )
}

/// Non-interactive element `<%element%>` cannot have interactive role '%role%'
pub fn a11y_no_noninteractive_element_to_interactive_role(element: &str, role: &str) -> String {
    format!(
        "Non-interactive element `<{element}>` cannot have interactive role '{role}'\nhttps://svelte.dev/e/a11y_no_noninteractive_element_to_interactive_role",
        element = element,
        role = role
    )
}

/// noninteractive element cannot have nonnegative tabIndex value
pub fn a11y_no_noninteractive_tabindex() -> String {
    format!(
        "noninteractive element cannot have nonnegative tabIndex value\nhttps://svelte.dev/e/a11y_no_noninteractive_tabindex"
    )
}

/// Redundant role '%role%'
pub fn a11y_no_redundant_roles(role: &str) -> String {
    format!(
        "Redundant role '{role}'\nhttps://svelte.dev/e/a11y_no_redundant_roles",
        role = role
    )
}

/// `<%element%>` with a %handler% handler must have an ARIA role
pub fn a11y_no_static_element_interactions(element: &str, handler: &str) -> String {
    format!(
        "`<{element}>` with a {handler} handler must have an ARIA role\nhttps://svelte.dev/e/a11y_no_static_element_interactions",
        element = element,
        handler = handler
    )
}

/// Avoid tabindex values above zero
pub fn a11y_positive_tabindex() -> String {
    format!("Avoid tabindex values above zero\nhttps://svelte.dev/e/a11y_positive_tabindex")
}

/// Elements with the ARIA role "%role%" must have the following attributes defined: %props%
pub fn a11y_role_has_required_aria_props(role: &str, props: &str) -> String {
    format!(
        "Elements with the ARIA role \"{role}\" must have the following attributes defined: {props}\nhttps://svelte.dev/e/a11y_role_has_required_aria_props",
        role = role,
        props = props
    )
}

/// The attribute '%attribute%' is not supported by the role '%role%'
pub fn a11y_role_supports_aria_props(attribute: &str, role: &str) -> String {
    format!(
        "The attribute '{attribute}' is not supported by the role '{role}'\nhttps://svelte.dev/e/a11y_role_supports_aria_props",
        attribute = attribute,
        role = role
    )
}

/// The attribute '%attribute%' is not supported by the role '%role%'. This role is implicit on the element `<%name%>`
pub fn a11y_role_supports_aria_props_implicit(attribute: &str, role: &str, name: &str) -> String {
    format!(
        "The attribute '{attribute}' is not supported by the role '{role}'. This role is implicit on the element `<{name}>`\nhttps://svelte.dev/e/a11y_role_supports_aria_props_implicit",
        attribute = attribute,
        role = role,
        name = name
    )
}

/// Unknown aria attribute 'aria-%attribute%'
pub fn a11y_unknown_aria_attribute(attribute: &str, suggestion: Option<&str>) -> String {
    if let Some(suggestion) = suggestion {
        format!(
            "Unknown aria attribute 'aria-{attribute}'. Did you mean '{suggestion}'?\nhttps://svelte.dev/e/a11y_unknown_aria_attribute",
            attribute = attribute,
            suggestion = suggestion
        )
    } else {
        format!(
            "Unknown aria attribute 'aria-{attribute}'\nhttps://svelte.dev/e/a11y_unknown_aria_attribute",
            attribute = attribute
        )
    }
}

/// Unknown role '%role%'
pub fn a11y_unknown_role(role: &str, suggestion: Option<&str>) -> String {
    if let Some(suggestion) = suggestion {
        format!(
            "Unknown role '{role}'. Did you mean '{suggestion}'?\nhttps://svelte.dev/e/a11y_unknown_role",
            role = role,
            suggestion = suggestion
        )
    } else {
        format!(
            "Unknown role '{role}'\nhttps://svelte.dev/e/a11y_unknown_role",
            role = role
        )
    }
}

/// The "is" attribute is not supported cross-browser and should be avoided
pub fn attribute_avoid_is() -> String {
    format!(
        "The \"is\" attribute is not supported cross-browser and should be avoided\nhttps://svelte.dev/e/attribute_avoid_is"
    )
}

/// You are referencing `globalThis.%name%`. Did you forget to declare a variable with that name?
pub fn attribute_global_event_reference(name: &str) -> String {
    format!(
        "You are referencing `globalThis.{name}`. Did you forget to declare a variable with that name?\nhttps://svelte.dev/e/attribute_global_event_reference",
        name = name
    )
}

/// Attributes should not contain ':' characters to prevent ambiguity with Svelte directives
pub fn attribute_illegal_colon() -> String {
    format!(
        "Attributes should not contain ':' characters to prevent ambiguity with Svelte directives\nhttps://svelte.dev/e/attribute_illegal_colon"
    )
}

/// '%wrong%' is not a valid HTML attribute. Did you mean '%right%'?
pub fn attribute_invalid_property_name(wrong: &str, right: &str) -> String {
    format!(
        "'{wrong}' is not a valid HTML attribute. Did you mean '{right}'?\nhttps://svelte.dev/e/attribute_invalid_property_name",
        wrong = wrong,
        right = right
    )
}

/// Quoted attributes on components and custom elements will be stringified in a future version of Svelte. If this isn't what you want, remove the quotes
pub fn attribute_quoted() -> String {
    format!(
        "Quoted attributes on components and custom elements will be stringified in a future version of Svelte. If this isn't what you want, remove the quotes\nhttps://svelte.dev/e/attribute_quoted"
    )
}

/// A bidirectional control character was detected in your code. These characters can be used to alter the visual direction of your code and could have unintended consequences
pub fn bidirectional_control_characters() -> String {
    format!(
        "A bidirectional control character was detected in your code. These characters can be used to alter the visual direction of your code and could have unintended consequences\nhttps://svelte.dev/e/bidirectional_control_characters"
    )
}

/// The rest operator (...) will create a new object and binding '%name%' with the original object will not work
pub fn bind_invalid_each_rest(name: &str) -> String {
    format!(
        "The rest operator (...) will create a new object and binding '{name}' with the original object will not work\nhttps://svelte.dev/e/bind_invalid_each_rest",
        name = name
    )
}

/// Empty block
pub fn block_empty() -> String {
    format!("Empty block\nhttps://svelte.dev/e/block_empty")
}

/// `<%name%>` will be treated as an HTML element unless it begins with a capital letter
pub fn component_name_lowercase(name: &str) -> String {
    format!(
        "`<{name}>` will be treated as an HTML element unless it begins with a capital letter\nhttps://svelte.dev/e/component_name_lowercase",
        name = name
    )
}

/// Unused CSS selector "%name%"
pub fn css_unused_selector(name: &str) -> String {
    format!(
        "Unused CSS selector \"{name}\"\nhttps://svelte.dev/e/css_unused_selector",
        name = name
    )
}

/// Using a rest element or a non-destructured declaration with `$props()` means that Svelte can't infer what properties to expose when creating a custom element. Consider destructuring all the props or explicitly specifying the `customElement.props` option.
pub fn custom_element_props_identifier() -> String {
    format!(
        "Using a rest element or a non-destructured declaration with `$props()` means that Svelte can't infer what properties to expose when creating a custom element. Consider destructuring all the props or explicitly specifying the `customElement.props` option.\nhttps://svelte.dev/e/custom_element_props_identifier"
    )
}

/// This element is implicitly closed by the following `%tag%`, which can cause an unexpected DOM structure. Add an explicit `%closing%` to avoid surprises.
pub fn element_implicitly_closed(tag: &str, closing: &str) -> String {
    format!(
        "This element is implicitly closed by the following `{tag}`, which can cause an unexpected DOM structure. Add an explicit `{closing}` to avoid surprises.\nhttps://svelte.dev/e/element_implicitly_closed",
        tag = tag,
        closing = closing
    )
}

/// Self-closing HTML tags for non-void elements are ambiguous — use `<%name% ...></%name%>` rather than `<%name% ... />`
pub fn element_invalid_self_closing_tag(name: &str) -> String {
    format!(
        "Self-closing HTML tags for non-void elements are ambiguous — use `<{name} ...></{name}>` rather than `<{name} ... />`\nhttps://svelte.dev/e/element_invalid_self_closing_tag",
        name = name
    )
}

/// Using `on:%name%` to listen to the %name% event is deprecated. Use the event attribute `on%name%` instead
pub fn event_directive_deprecated(name: &str) -> String {
    format!(
        "Using `on:{name}` to listen to the {name} event is deprecated. Use the event attribute `on{name}` instead\nhttps://svelte.dev/e/event_directive_deprecated",
        name = name
    )
}

/// Component has unused export property '%name%'. If it is for external reference only, please consider using `export const %name%`
pub fn export_let_unused(name: &str) -> String {
    format!(
        "Component has unused export property '{name}'. If it is for external reference only, please consider using `export const {name}`\nhttps://svelte.dev/e/export_let_unused",
        name = name
    )
}

/// `%code%` is no longer valid — please use `%suggestion%` instead
pub fn legacy_code(code: &str, suggestion: &str) -> String {
    format!(
        "`{code}` is no longer valid — please use `{suggestion}` instead\nhttps://svelte.dev/e/legacy_code",
        code = code,
        suggestion = suggestion
    )
}

/// Svelte 5 components are no longer classes. Instantiate them using `mount` or `hydrate` (imported from 'svelte') instead.
pub fn legacy_component_creation() -> String {
    format!(
        "Svelte 5 components are no longer classes. Instantiate them using `mount` or `hydrate` (imported from 'svelte') instead.\nhttps://svelte.dev/e/legacy_component_creation"
    )
}

/// %message%. When rendering this component on the server, the resulting HTML will be modified by the browser (by moving, removing, or inserting elements), likely resulting in a `hydration_mismatch` warning
pub fn node_invalid_placement_ssr(message: &str) -> String {
    format!(
        "{message}. When rendering this component on the server, the resulting HTML will be modified by the browser (by moving, removing, or inserting elements), likely resulting in a `hydration_mismatch` warning\nhttps://svelte.dev/e/node_invalid_placement_ssr",
        message = message
    )
}

/// `%name%` is updated, but is not declared with `$state(...)`. Changing its value will not correctly trigger updates
pub fn non_reactive_update(name: &str) -> String {
    format!(
        "`{name}` is updated, but is not declared with `$state(...)`. Changing its value will not correctly trigger updates\nhttps://svelte.dev/e/non_reactive_update",
        name = name
    )
}

/// The `accessors` option has been deprecated. It will have no effect in runes mode
pub fn options_deprecated_accessors() -> String {
    format!(
        "The `accessors` option has been deprecated. It will have no effect in runes mode\nhttps://svelte.dev/e/options_deprecated_accessors"
    )
}

/// The `immutable` option has been deprecated. It will have no effect in runes mode
pub fn options_deprecated_immutable() -> String {
    format!(
        "The `immutable` option has been deprecated. It will have no effect in runes mode\nhttps://svelte.dev/e/options_deprecated_immutable"
    )
}

/// The `customElement` option is used when generating a custom element. Did you forget the `customElement: true` compile option?
pub fn options_missing_custom_element() -> String {
    format!(
        "The `customElement` option is used when generating a custom element. Did you forget the `customElement: true` compile option?\nhttps://svelte.dev/e/options_missing_custom_element"
    )
}

/// The `enableSourcemap` option has been removed. Source maps are always generated now, and tooling can choose to ignore them
pub fn options_removed_enable_sourcemap() -> String {
    format!(
        "The `enableSourcemap` option has been removed. Source maps are always generated now, and tooling can choose to ignore them\nhttps://svelte.dev/e/options_removed_enable_sourcemap"
    )
}

/// The `hydratable` option has been removed. Svelte components are always hydratable now
pub fn options_removed_hydratable() -> String {
    format!(
        "The `hydratable` option has been removed. Svelte components are always hydratable now\nhttps://svelte.dev/e/options_removed_hydratable"
    )
}

/// The `loopGuardTimeout` option has been removed
pub fn options_removed_loop_guard_timeout() -> String {
    format!(
        "The `loopGuardTimeout` option has been removed\nhttps://svelte.dev/e/options_removed_loop_guard_timeout"
    )
}

/// `generate: "dom"` and `generate: "ssr"` options have been renamed to "client" and "server" respectively
pub fn options_renamed_ssr_dom() -> String {
    format!(
        "`generate: \"dom\"` and `generate: \"ssr\"` options have been renamed to \"client\" and \"server\" respectively\nhttps://svelte.dev/e/options_renamed_ssr_dom"
    )
}

/// Avoid 'new class' — instead, declare the class at the top level scope
pub fn perf_avoid_inline_class() -> String {
    format!(
        "Avoid 'new class' — instead, declare the class at the top level scope\nhttps://svelte.dev/e/perf_avoid_inline_class"
    )
}

/// Avoid declaring classes below the top level scope
pub fn perf_avoid_nested_class() -> String {
    format!(
        "Avoid declaring classes below the top level scope\nhttps://svelte.dev/e/perf_avoid_nested_class"
    )
}

/// Reactive declarations only exist at the top level of the instance script
pub fn reactive_declaration_invalid_placement() -> String {
    format!(
        "Reactive declarations only exist at the top level of the instance script\nhttps://svelte.dev/e/reactive_declaration_invalid_placement"
    )
}

/// Reassignments of module-level declarations will not cause reactive statements to update
pub fn reactive_declaration_module_script_dependency() -> String {
    format!(
        "Reassignments of module-level declarations will not cause reactive statements to update\nhttps://svelte.dev/e/reactive_declaration_module_script_dependency"
    )
}

/// `context="module"` is deprecated, use the `module` attribute instead
pub fn script_context_deprecated() -> String {
    format!(
        "`context=\"module\"` is deprecated, use the `module` attribute instead\nhttps://svelte.dev/e/script_context_deprecated"
    )
}

/// Unrecognized attribute — should be one of `generics`, `lang` or `module`. If this exists for a preprocessor, ensure that the preprocessor removes it
pub fn script_unknown_attribute() -> String {
    format!(
        "Unrecognized attribute — should be one of `generics`, `lang` or `module`. If this exists for a preprocessor, ensure that the preprocessor removes it\nhttps://svelte.dev/e/script_unknown_attribute"
    )
}

/// Using `<slot>` to render parent content is deprecated. Use `{@render ...}` tags instead
pub fn slot_element_deprecated() -> String {
    format!(
        "Using `<slot>` to render parent content is deprecated. Use `{{@render ...}}` tags instead\nhttps://svelte.dev/e/slot_element_deprecated"
    )
}

/// This reference only captures the initial value of `%name%`. Did you mean to reference it inside a %type% instead?
pub fn state_referenced_locally(name: &str, type_: &str) -> String {
    format!(
        "This reference only captures the initial value of `{name}`. Did you mean to reference it inside a {type_} instead?\nhttps://svelte.dev/e/state_referenced_locally",
        name = name,
        type_ = type_
    )
}

/// It looks like you're using the `$%name%` rune, but there is a local binding called `%name%`. Referencing a local variable with a `$` prefix will create a store subscription. Please rename `%name%` to avoid the ambiguity
pub fn store_rune_conflict(name: &str) -> String {
    format!(
        "It looks like you're using the `${name}` rune, but there is a local binding called `{name}`. Referencing a local variable with a `$` prefix will create a store subscription. Please rename `{name}` to avoid the ambiguity\nhttps://svelte.dev/e/store_rune_conflict",
        name = name
    )
}

/// `<svelte:component>` is deprecated in runes mode — components are dynamic by default
pub fn svelte_component_deprecated() -> String {
    format!(
        "`<svelte:component>` is deprecated in runes mode — components are dynamic by default\nhttps://svelte.dev/e/svelte_component_deprecated"
    )
}

/// `this` should be an `{expression}`. Using a string attribute value will cause an error in future versions of Svelte
pub fn svelte_element_invalid_this() -> String {
    format!(
        "`this` should be an `{{expression}}`. Using a string attribute value will cause an error in future versions of Svelte\nhttps://svelte.dev/e/svelte_element_invalid_this"
    )
}

/// `<svelte:self>` is deprecated — use self-imports (e.g. `import %name% from './%basename%'`) instead
pub fn svelte_self_deprecated(name: &str, basename: &str) -> String {
    format!(
        "`<svelte:self>` is deprecated — use self-imports (e.g. `import {name} from './{basename}'`) instead\nhttps://svelte.dev/e/svelte_self_deprecated",
        name = name,
        basename = basename
    )
}

/// `%code%` is not a recognised code
pub fn unknown_code(code: &str, suggestion: Option<&str>) -> String {
    if let Some(suggestion) = suggestion {
        format!(
            "`{code}` is not a recognised code (did you mean `{suggestion}`?)\nhttps://svelte.dev/e/unknown_code",
            code = code,
            suggestion = suggestion
        )
    } else {
        format!(
            "`{code}` is not a recognised code\nhttps://svelte.dev/e/unknown_code",
            code = code
        )
    }
}
