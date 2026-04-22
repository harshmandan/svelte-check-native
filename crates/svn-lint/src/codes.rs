// GENERATED — do not edit. Run `cargo run -p xtask --bin regen-lint-catalog`.
//
// Source: .svelte-upstream/svelte/packages/svelte/messages/compile-warnings/*.md

#![allow(non_camel_case_types)]

/// All known compile-warning codes from `svelte/compiler`.
///
/// Variant name matches the upstream snake_case code verbatim so the
/// `as_str` round-trip is trivial and generated-code scope is tiny.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Code {
    a11y_accesskey,
    a11y_aria_activedescendant_has_tabindex,
    a11y_aria_attributes,
    a11y_autocomplete_valid,
    a11y_autofocus,
    a11y_click_events_have_key_events,
    a11y_consider_explicit_label,
    a11y_distracting_elements,
    a11y_figcaption_index,
    a11y_figcaption_parent,
    a11y_hidden,
    a11y_img_redundant_alt,
    a11y_incorrect_aria_attribute_type,
    a11y_incorrect_aria_attribute_type_boolean,
    a11y_incorrect_aria_attribute_type_id,
    a11y_incorrect_aria_attribute_type_idlist,
    a11y_incorrect_aria_attribute_type_integer,
    a11y_incorrect_aria_attribute_type_token,
    a11y_incorrect_aria_attribute_type_tokenlist,
    a11y_incorrect_aria_attribute_type_tristate,
    a11y_interactive_supports_focus,
    a11y_invalid_attribute,
    a11y_label_has_associated_control,
    a11y_media_has_caption,
    a11y_misplaced_role,
    a11y_misplaced_scope,
    a11y_missing_attribute,
    a11y_missing_content,
    a11y_mouse_events_have_key_events,
    a11y_no_abstract_role,
    a11y_no_interactive_element_to_noninteractive_role,
    a11y_no_noninteractive_element_interactions,
    a11y_no_noninteractive_element_to_interactive_role,
    a11y_no_noninteractive_tabindex,
    a11y_no_redundant_roles,
    a11y_no_static_element_interactions,
    a11y_positive_tabindex,
    a11y_role_has_required_aria_props,
    a11y_role_supports_aria_props,
    a11y_role_supports_aria_props_implicit,
    a11y_unknown_aria_attribute,
    a11y_unknown_role,
    attribute_avoid_is,
    attribute_global_event_reference,
    attribute_illegal_colon,
    attribute_invalid_property_name,
    attribute_quoted,
    bidirectional_control_characters,
    bind_invalid_each_rest,
    block_empty,
    component_name_lowercase,
    css_unused_selector,
    custom_element_props_identifier,
    element_implicitly_closed,
    element_invalid_self_closing_tag,
    event_directive_deprecated,
    export_let_unused,
    legacy_code,
    legacy_component_creation,
    node_invalid_placement_ssr,
    non_reactive_update,
    options_deprecated_accessors,
    options_deprecated_immutable,
    options_missing_custom_element,
    options_removed_enable_sourcemap,
    options_removed_hydratable,
    options_removed_loop_guard_timeout,
    options_renamed_ssr_dom,
    perf_avoid_inline_class,
    perf_avoid_nested_class,
    reactive_declaration_invalid_placement,
    reactive_declaration_module_script_dependency,
    script_context_deprecated,
    script_unknown_attribute,
    slot_element_deprecated,
    state_referenced_locally,
    store_rune_conflict,
    svelte_component_deprecated,
    svelte_element_invalid_this,
    svelte_self_deprecated,
    unknown_code,
}

impl Code {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::a11y_accesskey => "a11y_accesskey",
            Self::a11y_aria_activedescendant_has_tabindex => {
                "a11y_aria_activedescendant_has_tabindex"
            }
            Self::a11y_aria_attributes => "a11y_aria_attributes",
            Self::a11y_autocomplete_valid => "a11y_autocomplete_valid",
            Self::a11y_autofocus => "a11y_autofocus",
            Self::a11y_click_events_have_key_events => "a11y_click_events_have_key_events",
            Self::a11y_consider_explicit_label => "a11y_consider_explicit_label",
            Self::a11y_distracting_elements => "a11y_distracting_elements",
            Self::a11y_figcaption_index => "a11y_figcaption_index",
            Self::a11y_figcaption_parent => "a11y_figcaption_parent",
            Self::a11y_hidden => "a11y_hidden",
            Self::a11y_img_redundant_alt => "a11y_img_redundant_alt",
            Self::a11y_incorrect_aria_attribute_type => "a11y_incorrect_aria_attribute_type",
            Self::a11y_incorrect_aria_attribute_type_boolean => {
                "a11y_incorrect_aria_attribute_type_boolean"
            }
            Self::a11y_incorrect_aria_attribute_type_id => "a11y_incorrect_aria_attribute_type_id",
            Self::a11y_incorrect_aria_attribute_type_idlist => {
                "a11y_incorrect_aria_attribute_type_idlist"
            }
            Self::a11y_incorrect_aria_attribute_type_integer => {
                "a11y_incorrect_aria_attribute_type_integer"
            }
            Self::a11y_incorrect_aria_attribute_type_token => {
                "a11y_incorrect_aria_attribute_type_token"
            }
            Self::a11y_incorrect_aria_attribute_type_tokenlist => {
                "a11y_incorrect_aria_attribute_type_tokenlist"
            }
            Self::a11y_incorrect_aria_attribute_type_tristate => {
                "a11y_incorrect_aria_attribute_type_tristate"
            }
            Self::a11y_interactive_supports_focus => "a11y_interactive_supports_focus",
            Self::a11y_invalid_attribute => "a11y_invalid_attribute",
            Self::a11y_label_has_associated_control => "a11y_label_has_associated_control",
            Self::a11y_media_has_caption => "a11y_media_has_caption",
            Self::a11y_misplaced_role => "a11y_misplaced_role",
            Self::a11y_misplaced_scope => "a11y_misplaced_scope",
            Self::a11y_missing_attribute => "a11y_missing_attribute",
            Self::a11y_missing_content => "a11y_missing_content",
            Self::a11y_mouse_events_have_key_events => "a11y_mouse_events_have_key_events",
            Self::a11y_no_abstract_role => "a11y_no_abstract_role",
            Self::a11y_no_interactive_element_to_noninteractive_role => {
                "a11y_no_interactive_element_to_noninteractive_role"
            }
            Self::a11y_no_noninteractive_element_interactions => {
                "a11y_no_noninteractive_element_interactions"
            }
            Self::a11y_no_noninteractive_element_to_interactive_role => {
                "a11y_no_noninteractive_element_to_interactive_role"
            }
            Self::a11y_no_noninteractive_tabindex => "a11y_no_noninteractive_tabindex",
            Self::a11y_no_redundant_roles => "a11y_no_redundant_roles",
            Self::a11y_no_static_element_interactions => "a11y_no_static_element_interactions",
            Self::a11y_positive_tabindex => "a11y_positive_tabindex",
            Self::a11y_role_has_required_aria_props => "a11y_role_has_required_aria_props",
            Self::a11y_role_supports_aria_props => "a11y_role_supports_aria_props",
            Self::a11y_role_supports_aria_props_implicit => {
                "a11y_role_supports_aria_props_implicit"
            }
            Self::a11y_unknown_aria_attribute => "a11y_unknown_aria_attribute",
            Self::a11y_unknown_role => "a11y_unknown_role",
            Self::attribute_avoid_is => "attribute_avoid_is",
            Self::attribute_global_event_reference => "attribute_global_event_reference",
            Self::attribute_illegal_colon => "attribute_illegal_colon",
            Self::attribute_invalid_property_name => "attribute_invalid_property_name",
            Self::attribute_quoted => "attribute_quoted",
            Self::bidirectional_control_characters => "bidirectional_control_characters",
            Self::bind_invalid_each_rest => "bind_invalid_each_rest",
            Self::block_empty => "block_empty",
            Self::component_name_lowercase => "component_name_lowercase",
            Self::css_unused_selector => "css_unused_selector",
            Self::custom_element_props_identifier => "custom_element_props_identifier",
            Self::element_implicitly_closed => "element_implicitly_closed",
            Self::element_invalid_self_closing_tag => "element_invalid_self_closing_tag",
            Self::event_directive_deprecated => "event_directive_deprecated",
            Self::export_let_unused => "export_let_unused",
            Self::legacy_code => "legacy_code",
            Self::legacy_component_creation => "legacy_component_creation",
            Self::node_invalid_placement_ssr => "node_invalid_placement_ssr",
            Self::non_reactive_update => "non_reactive_update",
            Self::options_deprecated_accessors => "options_deprecated_accessors",
            Self::options_deprecated_immutable => "options_deprecated_immutable",
            Self::options_missing_custom_element => "options_missing_custom_element",
            Self::options_removed_enable_sourcemap => "options_removed_enable_sourcemap",
            Self::options_removed_hydratable => "options_removed_hydratable",
            Self::options_removed_loop_guard_timeout => "options_removed_loop_guard_timeout",
            Self::options_renamed_ssr_dom => "options_renamed_ssr_dom",
            Self::perf_avoid_inline_class => "perf_avoid_inline_class",
            Self::perf_avoid_nested_class => "perf_avoid_nested_class",
            Self::reactive_declaration_invalid_placement => {
                "reactive_declaration_invalid_placement"
            }
            Self::reactive_declaration_module_script_dependency => {
                "reactive_declaration_module_script_dependency"
            }
            Self::script_context_deprecated => "script_context_deprecated",
            Self::script_unknown_attribute => "script_unknown_attribute",
            Self::slot_element_deprecated => "slot_element_deprecated",
            Self::state_referenced_locally => "state_referenced_locally",
            Self::store_rune_conflict => "store_rune_conflict",
            Self::svelte_component_deprecated => "svelte_component_deprecated",
            Self::svelte_element_invalid_this => "svelte_element_invalid_this",
            Self::svelte_self_deprecated => "svelte_self_deprecated",
            Self::unknown_code => "unknown_code",
        }
    }

    pub fn try_from_str(s: &str) -> Option<Self> {
        match s {
            "a11y_accesskey" => Some(Self::a11y_accesskey),
            "a11y_aria_activedescendant_has_tabindex" => {
                Some(Self::a11y_aria_activedescendant_has_tabindex)
            }
            "a11y_aria_attributes" => Some(Self::a11y_aria_attributes),
            "a11y_autocomplete_valid" => Some(Self::a11y_autocomplete_valid),
            "a11y_autofocus" => Some(Self::a11y_autofocus),
            "a11y_click_events_have_key_events" => Some(Self::a11y_click_events_have_key_events),
            "a11y_consider_explicit_label" => Some(Self::a11y_consider_explicit_label),
            "a11y_distracting_elements" => Some(Self::a11y_distracting_elements),
            "a11y_figcaption_index" => Some(Self::a11y_figcaption_index),
            "a11y_figcaption_parent" => Some(Self::a11y_figcaption_parent),
            "a11y_hidden" => Some(Self::a11y_hidden),
            "a11y_img_redundant_alt" => Some(Self::a11y_img_redundant_alt),
            "a11y_incorrect_aria_attribute_type" => Some(Self::a11y_incorrect_aria_attribute_type),
            "a11y_incorrect_aria_attribute_type_boolean" => {
                Some(Self::a11y_incorrect_aria_attribute_type_boolean)
            }
            "a11y_incorrect_aria_attribute_type_id" => {
                Some(Self::a11y_incorrect_aria_attribute_type_id)
            }
            "a11y_incorrect_aria_attribute_type_idlist" => {
                Some(Self::a11y_incorrect_aria_attribute_type_idlist)
            }
            "a11y_incorrect_aria_attribute_type_integer" => {
                Some(Self::a11y_incorrect_aria_attribute_type_integer)
            }
            "a11y_incorrect_aria_attribute_type_token" => {
                Some(Self::a11y_incorrect_aria_attribute_type_token)
            }
            "a11y_incorrect_aria_attribute_type_tokenlist" => {
                Some(Self::a11y_incorrect_aria_attribute_type_tokenlist)
            }
            "a11y_incorrect_aria_attribute_type_tristate" => {
                Some(Self::a11y_incorrect_aria_attribute_type_tristate)
            }
            "a11y_interactive_supports_focus" => Some(Self::a11y_interactive_supports_focus),
            "a11y_invalid_attribute" => Some(Self::a11y_invalid_attribute),
            "a11y_label_has_associated_control" => Some(Self::a11y_label_has_associated_control),
            "a11y_media_has_caption" => Some(Self::a11y_media_has_caption),
            "a11y_misplaced_role" => Some(Self::a11y_misplaced_role),
            "a11y_misplaced_scope" => Some(Self::a11y_misplaced_scope),
            "a11y_missing_attribute" => Some(Self::a11y_missing_attribute),
            "a11y_missing_content" => Some(Self::a11y_missing_content),
            "a11y_mouse_events_have_key_events" => Some(Self::a11y_mouse_events_have_key_events),
            "a11y_no_abstract_role" => Some(Self::a11y_no_abstract_role),
            "a11y_no_interactive_element_to_noninteractive_role" => {
                Some(Self::a11y_no_interactive_element_to_noninteractive_role)
            }
            "a11y_no_noninteractive_element_interactions" => {
                Some(Self::a11y_no_noninteractive_element_interactions)
            }
            "a11y_no_noninteractive_element_to_interactive_role" => {
                Some(Self::a11y_no_noninteractive_element_to_interactive_role)
            }
            "a11y_no_noninteractive_tabindex" => Some(Self::a11y_no_noninteractive_tabindex),
            "a11y_no_redundant_roles" => Some(Self::a11y_no_redundant_roles),
            "a11y_no_static_element_interactions" => {
                Some(Self::a11y_no_static_element_interactions)
            }
            "a11y_positive_tabindex" => Some(Self::a11y_positive_tabindex),
            "a11y_role_has_required_aria_props" => Some(Self::a11y_role_has_required_aria_props),
            "a11y_role_supports_aria_props" => Some(Self::a11y_role_supports_aria_props),
            "a11y_role_supports_aria_props_implicit" => {
                Some(Self::a11y_role_supports_aria_props_implicit)
            }
            "a11y_unknown_aria_attribute" => Some(Self::a11y_unknown_aria_attribute),
            "a11y_unknown_role" => Some(Self::a11y_unknown_role),
            "attribute_avoid_is" => Some(Self::attribute_avoid_is),
            "attribute_global_event_reference" => Some(Self::attribute_global_event_reference),
            "attribute_illegal_colon" => Some(Self::attribute_illegal_colon),
            "attribute_invalid_property_name" => Some(Self::attribute_invalid_property_name),
            "attribute_quoted" => Some(Self::attribute_quoted),
            "bidirectional_control_characters" => Some(Self::bidirectional_control_characters),
            "bind_invalid_each_rest" => Some(Self::bind_invalid_each_rest),
            "block_empty" => Some(Self::block_empty),
            "component_name_lowercase" => Some(Self::component_name_lowercase),
            "css_unused_selector" => Some(Self::css_unused_selector),
            "custom_element_props_identifier" => Some(Self::custom_element_props_identifier),
            "element_implicitly_closed" => Some(Self::element_implicitly_closed),
            "element_invalid_self_closing_tag" => Some(Self::element_invalid_self_closing_tag),
            "event_directive_deprecated" => Some(Self::event_directive_deprecated),
            "export_let_unused" => Some(Self::export_let_unused),
            "legacy_code" => Some(Self::legacy_code),
            "legacy_component_creation" => Some(Self::legacy_component_creation),
            "node_invalid_placement_ssr" => Some(Self::node_invalid_placement_ssr),
            "non_reactive_update" => Some(Self::non_reactive_update),
            "options_deprecated_accessors" => Some(Self::options_deprecated_accessors),
            "options_deprecated_immutable" => Some(Self::options_deprecated_immutable),
            "options_missing_custom_element" => Some(Self::options_missing_custom_element),
            "options_removed_enable_sourcemap" => Some(Self::options_removed_enable_sourcemap),
            "options_removed_hydratable" => Some(Self::options_removed_hydratable),
            "options_removed_loop_guard_timeout" => Some(Self::options_removed_loop_guard_timeout),
            "options_renamed_ssr_dom" => Some(Self::options_renamed_ssr_dom),
            "perf_avoid_inline_class" => Some(Self::perf_avoid_inline_class),
            "perf_avoid_nested_class" => Some(Self::perf_avoid_nested_class),
            "reactive_declaration_invalid_placement" => {
                Some(Self::reactive_declaration_invalid_placement)
            }
            "reactive_declaration_module_script_dependency" => {
                Some(Self::reactive_declaration_module_script_dependency)
            }
            "script_context_deprecated" => Some(Self::script_context_deprecated),
            "script_unknown_attribute" => Some(Self::script_unknown_attribute),
            "slot_element_deprecated" => Some(Self::slot_element_deprecated),
            "state_referenced_locally" => Some(Self::state_referenced_locally),
            "store_rune_conflict" => Some(Self::store_rune_conflict),
            "svelte_component_deprecated" => Some(Self::svelte_component_deprecated),
            "svelte_element_invalid_this" => Some(Self::svelte_element_invalid_this),
            "svelte_self_deprecated" => Some(Self::svelte_self_deprecated),
            "unknown_code" => Some(Self::unknown_code),
            _ => None,
        }
    }
}

/// All known codes, alphabetically sorted.
pub const CODES: &[&str; 81] = &[
    "a11y_accesskey",
    "a11y_aria_activedescendant_has_tabindex",
    "a11y_aria_attributes",
    "a11y_autocomplete_valid",
    "a11y_autofocus",
    "a11y_click_events_have_key_events",
    "a11y_consider_explicit_label",
    "a11y_distracting_elements",
    "a11y_figcaption_index",
    "a11y_figcaption_parent",
    "a11y_hidden",
    "a11y_img_redundant_alt",
    "a11y_incorrect_aria_attribute_type",
    "a11y_incorrect_aria_attribute_type_boolean",
    "a11y_incorrect_aria_attribute_type_id",
    "a11y_incorrect_aria_attribute_type_idlist",
    "a11y_incorrect_aria_attribute_type_integer",
    "a11y_incorrect_aria_attribute_type_token",
    "a11y_incorrect_aria_attribute_type_tokenlist",
    "a11y_incorrect_aria_attribute_type_tristate",
    "a11y_interactive_supports_focus",
    "a11y_invalid_attribute",
    "a11y_label_has_associated_control",
    "a11y_media_has_caption",
    "a11y_misplaced_role",
    "a11y_misplaced_scope",
    "a11y_missing_attribute",
    "a11y_missing_content",
    "a11y_mouse_events_have_key_events",
    "a11y_no_abstract_role",
    "a11y_no_interactive_element_to_noninteractive_role",
    "a11y_no_noninteractive_element_interactions",
    "a11y_no_noninteractive_element_to_interactive_role",
    "a11y_no_noninteractive_tabindex",
    "a11y_no_redundant_roles",
    "a11y_no_static_element_interactions",
    "a11y_positive_tabindex",
    "a11y_role_has_required_aria_props",
    "a11y_role_supports_aria_props",
    "a11y_role_supports_aria_props_implicit",
    "a11y_unknown_aria_attribute",
    "a11y_unknown_role",
    "attribute_avoid_is",
    "attribute_global_event_reference",
    "attribute_illegal_colon",
    "attribute_invalid_property_name",
    "attribute_quoted",
    "bidirectional_control_characters",
    "bind_invalid_each_rest",
    "block_empty",
    "component_name_lowercase",
    "css_unused_selector",
    "custom_element_props_identifier",
    "element_implicitly_closed",
    "element_invalid_self_closing_tag",
    "event_directive_deprecated",
    "export_let_unused",
    "legacy_code",
    "legacy_component_creation",
    "node_invalid_placement_ssr",
    "non_reactive_update",
    "options_deprecated_accessors",
    "options_deprecated_immutable",
    "options_missing_custom_element",
    "options_removed_enable_sourcemap",
    "options_removed_hydratable",
    "options_removed_loop_guard_timeout",
    "options_renamed_ssr_dom",
    "perf_avoid_inline_class",
    "perf_avoid_nested_class",
    "reactive_declaration_invalid_placement",
    "reactive_declaration_module_script_dependency",
    "script_context_deprecated",
    "script_unknown_attribute",
    "slot_element_deprecated",
    "state_referenced_locally",
    "store_rune_conflict",
    "svelte_component_deprecated",
    "svelte_element_invalid_this",
    "svelte_self_deprecated",
    "unknown_code",
];
