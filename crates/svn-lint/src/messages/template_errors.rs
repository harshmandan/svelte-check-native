//! Message texts of the compiler errors raised while validating the
//! template, verbatim from the compiler's `errors.js` (including the
//! trailing docs link).

#![allow(clippy::useless_format)]

/// An element can only have one 'animate' directive
pub fn animation_duplicate() -> String {
    format!(
        "An element can only have one 'animate' directive\nhttps://svelte.dev/e/animation_duplicate"
    )
}

/// An element that uses the `animate:` directive must be the only child of a keyed `{#each ...}` block
pub fn animation_invalid_placement() -> String {
    format!(
        "An element that uses the `animate:` directive must be the only child of a keyed `{{#each ...}}` block\nhttps://svelte.dev/e/animation_invalid_placement"
    )
}

/// An element that uses the `animate:` directive must be the only child of a keyed `{#each ...}` block. Did you forget to add a key to your each block?
pub fn animation_missing_key() -> String {
    format!(
        "An element that uses the `animate:` directive must be the only child of a keyed `{{#each ...}}` block. Did you forget to add a key to your each block?\nhttps://svelte.dev/e/animation_missing_key"
    )
}

/// 'contenteditable' attribute cannot be dynamic if element uses two-way binding
pub fn attribute_contenteditable_dynamic() -> String {
    format!(
        "'contenteditable' attribute cannot be dynamic if element uses two-way binding\nhttps://svelte.dev/e/attribute_contenteditable_dynamic"
    )
}

/// 'contenteditable' attribute is required for textContent, innerHTML and innerText two-way bindings
pub fn attribute_contenteditable_missing() -> String {
    format!(
        "'contenteditable' attribute is required for textContent, innerHTML and innerText two-way bindings\nhttps://svelte.dev/e/attribute_contenteditable_missing"
    )
}

/// 'multiple' attribute must be static if select uses two-way binding
pub fn attribute_invalid_multiple() -> String {
    format!(
        "'multiple' attribute must be static if select uses two-way binding\nhttps://svelte.dev/e/attribute_invalid_multiple"
    )
}

/// Comma-separated expressions are not allowed as attribute/directive values in runes mode, unless wrapped in parentheses
pub fn attribute_invalid_sequence_expression() -> String {
    format!(
        "Comma-separated expressions are not allowed as attribute/directive values in runes mode, unless wrapped in parentheses\nhttps://svelte.dev/e/attribute_invalid_sequence_expression"
    )
}

/// 'type' attribute must be a static text value if input uses two-way binding
pub fn attribute_invalid_type() -> String {
    format!(
        "'type' attribute must be a static text value if input uses two-way binding\nhttps://svelte.dev/e/attribute_invalid_type"
    )
}

/// Attribute values containing `{...}` must be enclosed in quote marks, unless the value only contains the expression
pub fn attribute_unquoted_sequence() -> String {
    format!(
        "Attribute values containing `{{...}}` must be enclosed in quote marks, unless the value only contains the expression\nhttps://svelte.dev/e/attribute_unquoted_sequence"
    )
}

/// `bind:group` can only bind to an Identifier or MemberExpression
pub fn bind_group_invalid_expression() -> String {
    format!(
        "`bind:group` can only bind to an Identifier or MemberExpression\nhttps://svelte.dev/e/bind_group_invalid_expression"
    )
}

/// Cannot `bind:group` to a snippet parameter
pub fn bind_group_invalid_snippet_parameter() -> String {
    format!(
        "Cannot `bind:group` to a snippet parameter\nhttps://svelte.dev/e/bind_group_invalid_snippet_parameter"
    )
}

/// Can only bind to an Identifier or MemberExpression or a `{get, set}` pair
pub fn bind_invalid_expression() -> String {
    format!(
        "Can only bind to an Identifier or MemberExpression or a `{{get, set}}` pair\nhttps://svelte.dev/e/bind_invalid_expression"
    )
}

/// `bind:%name%={get, set}` must not have surrounding parentheses
pub fn bind_invalid_parens(name: &str) -> String {
    format!(
        "`bind:{name}={{get, set}}` must not have surrounding parentheses\nhttps://svelte.dev/e/bind_invalid_parens"
    )
}

/// Expected a `%character%` character immediately following the opening bracket
pub fn block_unexpected_character(character: &str) -> String {
    format!(
        "Expected a `{character}` character immediately following the opening bracket\nhttps://svelte.dev/e/block_unexpected_character"
    )
}

/// `{@const}` must be the immediate child of `{#snippet}`, `{#if}`, `{:else if}`, `{:else}`, `{#each}`, `{:then}`, `{:catch}`, `<svelte:fragment>`, `<svelte:boundary>` or `<Component>`
pub fn const_tag_invalid_placement() -> String {
    format!(
        "`{{@const}}` must be the immediate child of `{{#snippet}}`, `{{#if}}`, `{{:else if}}`, `{{:else}}`, `{{#each}}`, `{{:then}}`, `{{:catch}}`, `<svelte:fragment>`, `<svelte:boundary>` or `<Component>`\nhttps://svelte.dev/e/const_tag_invalid_placement"
    )
}

/// An `{#each ...}` block without an `as` clause cannot have a key
pub fn each_key_without_as() -> String {
    format!(
        "An `{{#each ...}}` block without an `as` clause cannot have a key\nhttps://svelte.dev/e/each_key_without_as"
    )
}

/// Valid event modifiers are %list%
pub fn event_handler_invalid_modifier(list: &str) -> String {
    format!("Valid event modifiers are {list}\nhttps://svelte.dev/e/event_handler_invalid_modifier")
}

/// The '%modifier1%' and '%modifier2%' modifiers cannot be used together
pub fn event_handler_invalid_modifier_combination(modifier1: &str, modifier2: &str) -> String {
    format!(
        "The '{modifier1}' and '{modifier2}' modifiers cannot be used together\nhttps://svelte.dev/e/event_handler_invalid_modifier_combination"
    )
}

/// `use:`, `transition:` and `animate:` directives, attachments and bindings do not support await expressions
pub fn illegal_await_expression() -> String {
    format!(
        "`use:`, `transition:` and `animate:` directives, attachments and bindings do not support await expressions\nhttps://svelte.dev/e/illegal_await_expression"
    )
}

/// `<%name%>` does not support non-event attributes or spread attributes
pub fn illegal_element_attribute(name: &str) -> String {
    format!(
        "`<{name}>` does not support non-event attributes or spread attributes\nhttps://svelte.dev/e/illegal_element_attribute"
    )
}

/// `let:` directive at invalid position
pub fn let_directive_invalid_placement() -> String {
    format!(
        "`let:` directive at invalid position\nhttps://svelte.dev/e/let_directive_invalid_placement"
    )
}

/// Calling a snippet function using apply, bind or call is not allowed
pub fn render_tag_invalid_call_expression() -> String {
    format!(
        "Calling a snippet function using apply, bind or call is not allowed\nhttps://svelte.dev/e/render_tag_invalid_call_expression"
    )
}

/// cannot use spread arguments in `{@render ...}` tags
pub fn render_tag_invalid_spread_argument() -> String {
    format!(
        "cannot use spread arguments in `{{@render ...}}` tags\nhttps://svelte.dev/e/render_tag_invalid_spread_argument"
    )
}

/// Duplicate slot name '%name%' in <%component%>
pub fn slot_attribute_duplicate(name: &str, component: &str) -> String {
    format!(
        "Duplicate slot name '{name}' in <{component}>\nhttps://svelte.dev/e/slot_attribute_duplicate"
    )
}

/// Found default slot content alongside an explicit slot="default"
pub fn slot_default_duplicate() -> String {
    format!(
        "Found default slot content alongside an explicit slot=\"default\"\nhttps://svelte.dev/e/slot_default_duplicate"
    )
}

/// Snippets do not support rest parameters; use an array instead
pub fn snippet_invalid_rest_parameter() -> String {
    format!(
        "Snippets do not support rest parameters; use an array instead\nhttps://svelte.dev/e/snippet_invalid_rest_parameter"
    )
}

/// `style:` directive can only use the `important` modifier
pub fn style_directive_invalid_modifier() -> String {
    format!(
        "`style:` directive can only use the `important` modifier\nhttps://svelte.dev/e/style_directive_invalid_modifier"
    )
}

/// `<svelte:body>` does not support non-event attributes or spread attributes
pub fn svelte_body_illegal_attribute() -> String {
    format!(
        "`<svelte:body>` does not support non-event attributes or spread attributes\nhttps://svelte.dev/e/svelte_body_illegal_attribute"
    )
}

/// Valid attributes on `<svelte:boundary>` are `onerror` and `failed`
pub fn svelte_boundary_invalid_attribute() -> String {
    format!(
        "Valid attributes on `<svelte:boundary>` are `onerror` and `failed`\nhttps://svelte.dev/e/svelte_boundary_invalid_attribute"
    )
}

/// Attribute value must be a non-string expression
pub fn svelte_boundary_invalid_attribute_value() -> String {
    format!(
        "Attribute value must be a non-string expression\nhttps://svelte.dev/e/svelte_boundary_invalid_attribute_value"
    )
}

/// `<svelte:fragment>` can only have a slot attribute and (optionally) a let: directive
pub fn svelte_fragment_invalid_attribute() -> String {
    format!(
        "`<svelte:fragment>` can only have a slot attribute and (optionally) a let: directive\nhttps://svelte.dev/e/svelte_fragment_invalid_attribute"
    )
}

/// `<svelte:fragment>` must be the direct child of a component
pub fn svelte_fragment_invalid_placement() -> String {
    format!(
        "`<svelte:fragment>` must be the direct child of a component\nhttps://svelte.dev/e/svelte_fragment_invalid_placement"
    )
}

/// `<svelte:head>` cannot have attributes nor directives
pub fn svelte_head_illegal_attribute() -> String {
    format!(
        "`<svelte:head>` cannot have attributes nor directives\nhttps://svelte.dev/e/svelte_head_illegal_attribute"
    )
}

/// <%name%> cannot have children
pub fn svelte_meta_invalid_content(name: &str) -> String {
    format!("<{name}> cannot have children\nhttps://svelte.dev/e/svelte_meta_invalid_content")
}

/// A `<textarea>` can have either a value attribute or (equivalently) child content, but not both
pub fn textarea_invalid_content() -> String {
    format!(
        "A `<textarea>` can have either a value attribute or (equivalently) child content, but not both\nhttps://svelte.dev/e/textarea_invalid_content"
    )
}

/// `<title>` cannot have attributes nor directives
pub fn title_illegal_attribute() -> String {
    format!(
        "`<title>` cannot have attributes nor directives\nhttps://svelte.dev/e/title_illegal_attribute"
    )
}

/// `<title>` can only contain text and {tags}
pub fn title_invalid_content() -> String {
    format!(
        "`<title>` can only contain text and {{tags}}\nhttps://svelte.dev/e/title_invalid_content"
    )
}

/// Cannot use `%type%:` alongside existing `%existing%:` directive
pub fn transition_conflict(type_: &str, existing: &str) -> String {
    format!(
        "Cannot use `{type_}:` alongside existing `{existing}:` directive\nhttps://svelte.dev/e/transition_conflict"
    )
}

/// Cannot use multiple `%type%:` directives on a single element
pub fn transition_duplicate(type_: &str) -> String {
    format!(
        "Cannot use multiple `{type_}:` directives on a single element\nhttps://svelte.dev/e/transition_duplicate"
    )
}
