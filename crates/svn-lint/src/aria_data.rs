//! ARIA role tables. Vendored directly from `aria-query@5.*`'s
//! `roles` / `elementRoles` / `aria` maps.
//!
//! Generated once by the scripts at the top of Phase E; keep the
//! regeneration procedure in the handoff notes if aria-query bumps.
//! No runtime dependency on the npm package.
//!
//! Encoding notes:
//! - `ROLE_PROPS` stores each role's supported ARIA properties as a
//!   `u64` bitmask, with bit index = position in
//!   [`crate::a11y_constants::ARIA_PROPS`]. 48 aria props today,
//!   comfortably fits in `u64` (headroom for future expansion).
//! - `INTERACTIVE_ELEMENT_SCHEMAS` / `NON_INTERACTIVE_ELEMENT_SCHEMAS`
//!   encode element-role schema matches via `(tag name, attribute
//!   constraints)`. An attribute constraint matches either by name
//!   only (presence) or name+value.

/// One element-role schema entry. `attrs` is AND-joined: every
/// constraint must match for the schema to apply.
pub struct Schema {
    pub name: &'static str,
    pub attrs: &'static [(&'static str, Option<&'static str>)],
}

/// Resolve an ARIA prop name to its bit index in the role bitmasks.
/// Returns `None` for unknown names (shouldn't happen — aria_prop()
/// handles validation before this call).
pub fn aria_prop_bit(name: &str) -> Option<u32> {
    ARIA_PROP_BITS.get(name).copied()
}

/// Look up a role by name. Returns `(props_bitmask, required_props)`
/// if the role is known and non-abstract.
pub fn role_props(role: &str) -> Option<(u64, &'static [&'static str])> {
    ROLE_PROPS.get(role).copied()
}

/// True if `role` (assumed a known aria role name) supports `prop`.
/// `prop` must be the full `aria-*` name.
pub fn role_supports_prop(role: &str, prop: &str) -> bool {
    let Some((mask, _)) = role_props(role) else {
        return false;
    };
    let Some(bit) = aria_prop_bit(prop) else {
        return false;
    };
    (mask & (1u64 << bit)) != 0
}

/// Represents what the walker knows about an attribute: absent,
/// present-but-dynamic (or bare), or present with a literal string.
#[derive(Clone, Copy, Debug)]
pub enum AttrState<'a> {
    Absent,
    Dynamic,
    Literal(&'a str),
}

/// Does this element match *any* interactive-element-role schema?
pub fn is_interactive_element_schema<'a>(
    name: &str,
    get_attr: impl Fn(&str) -> AttrState<'a> + Copy,
) -> bool {
    INTERACTIVE_ELEMENT_SCHEMAS
        .iter()
        .any(|s| schema_matches(s, name, get_attr))
}

/// Does this element match any non-interactive-element-role schema?
pub fn is_non_interactive_element_schema<'a>(
    name: &str,
    get_attr: impl Fn(&str) -> AttrState<'a> + Copy,
) -> bool {
    NON_INTERACTIVE_ELEMENT_SCHEMAS
        .iter()
        .any(|s| schema_matches(s, name, get_attr))
}

/// Matches a single schema. Attribute semantics:
/// - `Some(val)` → attr must be present with literal value `val`
/// - `None` → attr must be present (any value, including dynamic).
fn schema_matches<'a>(
    schema: &Schema,
    name: &str,
    get_attr: impl Fn(&str) -> AttrState<'a>,
) -> bool {
    if schema.name != name {
        return false;
    }
    for (attr_name, expected) in schema.attrs {
        match (get_attr(attr_name), expected) {
            (AttrState::Absent, _) => return false,
            (_, None) => {}
            (AttrState::Literal(v), Some(want)) => {
                if v != *want {
                    return false;
                }
            }
            (AttrState::Dynamic, Some(_)) => return false,
        }
    }
    true
}

/// Bit index for each ARIA prop name in the [`ROLE_PROPS`] bitmasks.
/// Order MUST stay in sync with [`crate::a11y_constants::ARIA_PROPS`]
/// — the bitmasks below were generated from that ordering.
pub static ARIA_PROP_BITS: phf::Map<&'static str, u32> = phf::phf_map! {
    "aria-activedescendant" => 0,
    "aria-atomic" => 1,
    "aria-autocomplete" => 2,
    "aria-braillelabel" => 3,
    "aria-brailleroledescription" => 4,
    "aria-busy" => 5,
    "aria-checked" => 6,
    "aria-colcount" => 7,
    "aria-colindex" => 8,
    "aria-colspan" => 9,
    "aria-controls" => 10,
    "aria-current" => 11,
    "aria-describedby" => 12,
    "aria-description" => 13,
    "aria-details" => 14,
    "aria-disabled" => 15,
    "aria-dropeffect" => 16,
    "aria-errormessage" => 17,
    "aria-expanded" => 18,
    "aria-flowto" => 19,
    "aria-grabbed" => 20,
    "aria-haspopup" => 21,
    "aria-hidden" => 22,
    "aria-invalid" => 23,
    "aria-keyshortcuts" => 24,
    "aria-label" => 25,
    "aria-labelledby" => 26,
    "aria-level" => 27,
    "aria-live" => 28,
    "aria-modal" => 29,
    "aria-multiline" => 30,
    "aria-multiselectable" => 31,
    "aria-orientation" => 32,
    "aria-owns" => 33,
    "aria-placeholder" => 34,
    "aria-posinset" => 35,
    "aria-pressed" => 36,
    "aria-readonly" => 37,
    "aria-relevant" => 38,
    "aria-required" => 39,
    "aria-roledescription" => 40,
    "aria-rowcount" => 41,
    "aria-rowindex" => 42,
    "aria-rowspan" => 43,
    "aria-selected" => 44,
    "aria-setsize" => 45,
    "aria-sort" => 46,
    "aria-valuemax" => 47,
    "aria-valuemin" => 48,
    "aria-valuenow" => 49,
    "aria-valuetext" => 50,
};

/// Per-role (props_bitmask, required_props). Derived from
/// `aria-query@5.x`'s rolesMap — 127 non-abstract roles.
pub static ROLE_PROPS: phf::Map<&'static str, (u64, &'static [&'static str])> = phf::phf_map! {
    "alert" => (0x14217595c22u64, &[]),
    "alertdialog" => (0x14237595c22u64, &[]),
    "application" => (0x14217ffdc23u64, &[]),
    "article" => (0x214a17595c22u64, &[]),
    "banner" => (0x14217595c22u64, &[]),
    "blockquote" => (0x14217595c22u64, &[]),
    "button" => (0x152177ddc22u64, &[]),
    "caption" => (0x14217595c22u64, &[]),
    "cell" => (0xd4217595f22u64, &[]),
    "checkbox" => (0x1e217dfdc62u64, &["aria-checked"]),
    "code" => (0x14217595c22u64, &[]),
    "columnheader" => (0x5de217ffdf22u64, &[]),
    "combobox" => (0x1e217ffdc27u64, &["aria-controls", "aria-expanded"]),
    "complementary" => (0x14217595c22u64, &[]),
    "contentinfo" => (0x14217595c22u64, &[]),
    "definition" => (0x14217595c22u64, &[]),
    "deletion" => (0x14217595c22u64, &[]),
    "dialog" => (0x14237595c22u64, &[]),
    "directory" => (0x14217595c22u64, &[]),
    "doc-abstract" => (0x14217ffdc22u64, &[]),
    "doc-acknowledgments" => (0x14217ffdc22u64, &[]),
    "doc-afterword" => (0x14217ffdc22u64, &[]),
    "doc-appendix" => (0x14217ffdc22u64, &[]),
    "doc-backlink" => (0x14217ffdc22u64, &[]),
    "doc-biblioentry" => (0x214a1fffdc22u64, &[]),
    "doc-bibliography" => (0x14217ffdc22u64, &[]),
    "doc-biblioref" => (0x14217ffdc22u64, &[]),
    "doc-chapter" => (0x14217ffdc22u64, &[]),
    "doc-colophon" => (0x14217ffdc22u64, &[]),
    "doc-conclusion" => (0x14217ffdc22u64, &[]),
    "doc-cover" => (0x14217ffdc22u64, &[]),
    "doc-credit" => (0x14217ffdc22u64, &[]),
    "doc-credits" => (0x14217ffdc22u64, &[]),
    "doc-dedication" => (0x14217ffdc22u64, &[]),
    "doc-endnote" => (0x214a1fffdc22u64, &[]),
    "doc-endnotes" => (0x14217ffdc22u64, &[]),
    "doc-epigraph" => (0x14217ffdc22u64, &[]),
    "doc-epilogue" => (0x14217ffdc22u64, &[]),
    "doc-errata" => (0x14217ffdc22u64, &[]),
    "doc-example" => (0x14217ffdc22u64, &[]),
    "doc-footnote" => (0x14217ffdc22u64, &[]),
    "doc-foreword" => (0x14217ffdc22u64, &[]),
    "doc-glossary" => (0x14217ffdc22u64, &[]),
    "doc-glossref" => (0x14217ffdc22u64, &[]),
    "doc-index" => (0x14217ffdc22u64, &[]),
    "doc-introduction" => (0x14217ffdc22u64, &[]),
    "doc-noteref" => (0x14217ffdc22u64, &[]),
    "doc-notice" => (0x14217ffdc22u64, &[]),
    "doc-pagebreak" => (0x7814317ffdc22u64, &[]),
    "doc-pagefooter" => (0x14217fbfc3au64, &[]),
    "doc-pageheader" => (0x14217fbfc3au64, &[]),
    "doc-pagelist" => (0x14217ffdc22u64, &[]),
    "doc-part" => (0x14217ffdc22u64, &[]),
    "doc-preface" => (0x14217ffdc22u64, &[]),
    "doc-prologue" => (0x14217ffdc22u64, &[]),
    "doc-pullquote" => (0x0u64, &[]),
    "doc-qna" => (0x14217ffdc22u64, &[]),
    "doc-subtitle" => (0x14217ffdc22u64, &[]),
    "doc-tip" => (0x14217ffdc22u64, &[]),
    "doc-toc" => (0x14217ffdc22u64, &[]),
    "document" => (0x14217595c22u64, &[]),
    "emphasis" => (0x14217595c22u64, &[]),
    "feed" => (0x14217595c22u64, &[]),
    "figure" => (0x14217595c22u64, &[]),
    "form" => (0x14217595c22u64, &[]),
    "generic" => (0x14217595c22u64, &[]),
    "graphics-document" => (0x14217ffdc22u64, &[]),
    "graphics-object" => (0x14217ffdc23u64, &[]),
    "graphics-symbol" => (0x14217ffdc22u64, &[]),
    "grid" => (0x3629759dca3u64, &[]),
    "gridcell" => (0x1de217ffdf22u64, &[]),
    "group" => (0x1421759dc23u64, &[]),
    "heading" => (0x1421f595c22u64, &["aria-level"]),
    "img" => (0x14217595c22u64, &[]),
    "insertion" => (0x14217595c22u64, &[]),
    "link" => (0x142177ddc22u64, &[]),
    "list" => (0x14217595c22u64, &[]),
    "listbox" => (0x1e397dfdc23u64, &[]),
    "listitem" => (0x214a1f595c22u64, &[]),
    "log" => (0x14217595c22u64, &[]),
    "main" => (0x14217595c22u64, &[]),
    "mark" => (0x14217597c3au64, &[]),
    "marquee" => (0x14217595c22u64, &[]),
    "math" => (0x14217595c22u64, &[]),
    "menu" => (0x1431759dc23u64, &[]),
    "menubar" => (0x1431759dc23u64, &[]),
    "menuitem" => (0x214a177ddc22u64, &[]),
    "menuitemcheckbox" => (0x21ea17ffdc62u64, &["aria-checked"]),
    "menuitemradio" => (0x21ea17ffdc62u64, &["aria-checked"]),
    "meter" => (0x7814217595c22u64, &["aria-valuenow"]),
    "navigation" => (0x14217595c22u64, &[]),
    "none" => (0x0u64, &[]),
    "note" => (0x14217595c22u64, &[]),
    "option" => (0x314a1759dc62u64, &["aria-selected"]),
    "paragraph" => (0x14217595c22u64, &[]),
    "presentation" => (0x14217595c22u64, &[]),
    "progressbar" => (0x7814217595c22u64, &[]),
    "radio" => (0x214a1759dc62u64, &["aria-checked"]),
    "radiogroup" => (0x1e317dbdc23u64, &[]),
    "region" => (0x14217595c22u64, &[]),
    "row" => (0x354a1f5ddd23u64, &[]),
    "rowgroup" => (0x14217595c22u64, &[]),
    "rowheader" => (0x5de217ffdf22u64, &[]),
    "scrollbar" => (0x781431759dc22u64, &["aria-controls", "aria-valuenow"]),
    "search" => (0x14217595c22u64, &[]),
    "searchbox" => (0x1e657fbdc27u64, &[]),
    "separator" => (0x781431759dc22u64, &[]),
    "slider" => (0x7816317fbdc22u64, &["aria-valuenow"]),
    "spinbutton" => (0x781e217dbdc23u64, &[]),
    "status" => (0x14217595c22u64, &[]),
    "strong" => (0x14217595c22u64, &[]),
    "subscript" => (0x14217595c22u64, &[]),
    "superscript" => (0x14217595c22u64, &[]),
    "switch" => (0x1e217dfdc62u64, &["aria-checked"]),
    "tab" => (0x314a177ddc22u64, &[]),
    "table" => (0x34217595ca2u64, &[]),
    "tablist" => (0x1439f59dc23u64, &[]),
    "tabpanel" => (0x14217595c22u64, &[]),
    "term" => (0x14217595c22u64, &[]),
    "textbox" => (0x1e657fbdc27u64, &[]),
    "time" => (0x14217595c22u64, &[]),
    "timer" => (0x14217595c22u64, &[]),
    "toolbar" => (0x1431759dc23u64, &[]),
    "tooltip" => (0x14217595c22u64, &[]),
    "tree" => (0x1c397dbdc23u64, &[]),
    "treegrid" => (0x3e397dbdca3u64, &[]),
    "treeitem" => (0x314a1f7ddc62u64, &["aria-selected"]),
};

pub const INTERACTIVE_ELEMENT_SCHEMAS: &[Schema] = &[
    // role-interactive (derived from aria-query's elementRoles →
    // widget/window superClass)
    Schema {
        name: "input",
        attrs: &[("type", Some("button"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("image"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("reset"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("submit"))],
    },
    Schema {
        name: "button",
        attrs: &[],
    },
    Schema {
        name: "td",
        attrs: &[],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("checkbox"))],
    },
    Schema {
        name: "th",
        attrs: &[],
    },
    Schema {
        name: "th",
        attrs: &[("scope", Some("col"))],
    },
    Schema {
        name: "th",
        attrs: &[("scope", Some("colgroup"))],
    },
    Schema {
        name: "input",
        attrs: &[("list", None), ("type", Some("email"))],
    },
    Schema {
        name: "input",
        attrs: &[("list", None), ("type", Some("search"))],
    },
    Schema {
        name: "input",
        attrs: &[("list", None), ("type", Some("tel"))],
    },
    Schema {
        name: "input",
        attrs: &[("list", None), ("type", Some("text"))],
    },
    Schema {
        name: "input",
        attrs: &[("list", None), ("type", Some("url"))],
    },
    Schema {
        name: "select",
        attrs: &[],
    },
    Schema {
        name: "dialog",
        attrs: &[],
    },
    Schema {
        name: "td",
        attrs: &[],
    },
    Schema {
        name: "a",
        attrs: &[("href", None)],
    },
    Schema {
        name: "area",
        attrs: &[("href", None)],
    },
    Schema {
        name: "select",
        attrs: &[("size", None)],
    },
    Schema {
        name: "select",
        attrs: &[("multiple", None)],
    },
    Schema {
        name: "datalist",
        attrs: &[],
    },
    Schema {
        name: "option",
        attrs: &[],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("radio"))],
    },
    Schema {
        name: "tr",
        attrs: &[],
    },
    Schema {
        name: "th",
        attrs: &[("scope", Some("row"))],
    },
    Schema {
        name: "th",
        attrs: &[("scope", Some("rowgroup"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("search"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("range"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("number"))],
    },
    Schema {
        name: "input",
        attrs: &[],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("email"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("tel"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("text"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("url"))],
    },
    Schema {
        name: "textarea",
        attrs: &[],
    },
    // ax-interactive (axobject-query elementAXObjects → widget
    // AXObject type) — additional members not covered above.
    Schema {
        name: "audio",
        attrs: &[],
    },
    Schema {
        name: "canvas",
        attrs: &[],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("color"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("date"))],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("datetime"))],
    },
    Schema {
        name: "summary",
        attrs: &[],
    },
    Schema {
        name: "embed",
        attrs: &[],
    },
    Schema {
        name: "input",
        attrs: &[("type", Some("time"))],
    },
    Schema {
        name: "menuitem",
        attrs: &[],
    },
    Schema {
        name: "video",
        attrs: &[],
    },
];

/// (schema, roles) pairs from `axobject-query`'s `elementAXObjects`
/// mapped through `AXObjectRoles` to their exposed ARIA role names.
/// Used by `a11y_role_has_required_aria_props` to suppress fires when
/// an element natively exposes the role (`<input type="checkbox">`
/// already provides the `switch` role and hence `aria-checked` isn't
/// actually missing).
pub const ELEMENT_AX_ROLES: &[(Schema, &[&str])] = &[
    (
        Schema {
            name: "article",
            attrs: &[],
        },
        &["article"],
    ),
    (
        Schema {
            name: "button",
            attrs: &[],
        },
        &["button"],
    ),
    (
        Schema {
            name: "td",
            attrs: &[],
        },
        &["cell", "gridcell"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[("type", Some("checkbox"))],
        },
        &["checkbox", "switch"],
    ),
    (
        Schema {
            name: "th",
            attrs: &[],
        },
        &["columnheader"],
    ),
    (
        Schema {
            name: "select",
            attrs: &[],
        },
        &["combobox", "listbox"],
    ),
    (
        Schema {
            name: "dialog",
            attrs: &[],
        },
        &["dialog"],
    ),
    (
        Schema {
            name: "dir",
            attrs: &[],
        },
        &["directory"],
    ),
    (
        Schema {
            name: "figure",
            attrs: &[],
        },
        &["figure"],
    ),
    (
        Schema {
            name: "form",
            attrs: &[],
        },
        &["form"],
    ),
    (
        Schema {
            name: "h1",
            attrs: &[],
        },
        &["heading"],
    ),
    (
        Schema {
            name: "h2",
            attrs: &[],
        },
        &["heading"],
    ),
    (
        Schema {
            name: "h3",
            attrs: &[],
        },
        &["heading"],
    ),
    (
        Schema {
            name: "h4",
            attrs: &[],
        },
        &["heading"],
    ),
    (
        Schema {
            name: "h5",
            attrs: &[],
        },
        &["heading"],
    ),
    (
        Schema {
            name: "h6",
            attrs: &[],
        },
        &["heading"],
    ),
    (
        Schema {
            name: "img",
            attrs: &[],
        },
        &["img"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[],
        },
        &["textbox"],
    ),
    (
        Schema {
            name: "a",
            attrs: &[("href", None)],
        },
        &["link"],
    ),
    (
        Schema {
            name: "option",
            attrs: &[],
        },
        &["option"],
    ),
    (
        Schema {
            name: "datalist",
            attrs: &[],
        },
        &["listbox"],
    ),
    (
        Schema {
            name: "li",
            attrs: &[],
        },
        &["listitem"],
    ),
    (
        Schema {
            name: "ul",
            attrs: &[],
        },
        &["list"],
    ),
    (
        Schema {
            name: "ol",
            attrs: &[],
        },
        &["list"],
    ),
    (
        Schema {
            name: "main",
            attrs: &[],
        },
        &["main"],
    ),
    (
        Schema {
            name: "marquee",
            attrs: &[],
        },
        &["marquee"],
    ),
    (
        Schema {
            name: "menuitem",
            attrs: &[],
        },
        &["menuitem"],
    ),
    (
        Schema {
            name: "menu",
            attrs: &[],
        },
        &["menu"],
    ),
    (
        Schema {
            name: "nav",
            attrs: &[],
        },
        &["navigation"],
    ),
    (
        Schema {
            name: "progress",
            attrs: &[],
        },
        &["progressbar"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[("type", Some("radio"))],
        },
        &["radio"],
    ),
    (
        Schema {
            name: "th",
            attrs: &[("scope", Some("row"))],
        },
        &["rowheader"],
    ),
    (
        Schema {
            name: "tr",
            attrs: &[],
        },
        &["row"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[("type", Some("search"))],
        },
        &["searchbox"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[("type", Some("range"))],
        },
        &["slider"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[("type", Some("number"))],
        },
        &["spinbutton"],
    ),
    (
        Schema {
            name: "table",
            attrs: &[],
        },
        &["table"],
    ),
    (
        Schema {
            name: "textarea",
            attrs: &[],
        },
        &["textbox"],
    ),
    (
        Schema {
            name: "input",
            attrs: &[("type", Some("text"))],
        },
        &["textbox"],
    ),
];

/// Does the native element schema expose `role` as one of its AX
/// roles? Mirrors upstream `is_semantic_role_element` — when true,
/// `a11y_role_has_required_aria_props` silently skips because the
/// element already provides the role natively.
pub fn is_semantic_role_element<'a>(
    role: &str,
    name: &str,
    get_attr: impl Fn(&str) -> AttrState<'a> + Copy,
) -> bool {
    for (schema, roles) in ELEMENT_AX_ROLES {
        if schema_matches(schema, name, get_attr) && roles.contains(&role) {
            return true;
        }
    }
    false
}

pub const NON_INTERACTIVE_ELEMENT_SCHEMAS: &[Schema] = &[
    Schema {
        name: "article",
        attrs: &[],
    },
    Schema {
        name: "header",
        attrs: &[],
    },
    Schema {
        name: "blockquote",
        attrs: &[],
    },
    Schema {
        name: "caption",
        attrs: &[],
    },
    Schema {
        name: "code",
        attrs: &[],
    },
    Schema {
        name: "aside",
        attrs: &[],
    },
    Schema {
        name: "aside",
        attrs: &[("aria-label", None)],
    },
    Schema {
        name: "aside",
        attrs: &[("aria-labelledby", None)],
    },
    Schema {
        name: "footer",
        attrs: &[],
    },
    Schema {
        name: "dd",
        attrs: &[],
    },
    Schema {
        name: "del",
        attrs: &[],
    },
    Schema {
        name: "html",
        attrs: &[],
    },
    Schema {
        name: "em",
        attrs: &[],
    },
    Schema {
        name: "figure",
        attrs: &[],
    },
    Schema {
        name: "form",
        attrs: &[("aria-label", None)],
    },
    Schema {
        name: "form",
        attrs: &[("aria-labelledby", None)],
    },
    Schema {
        name: "form",
        attrs: &[("name", None)],
    },
    Schema {
        name: "details",
        attrs: &[],
    },
    Schema {
        name: "fieldset",
        attrs: &[],
    },
    Schema {
        name: "optgroup",
        attrs: &[],
    },
    Schema {
        name: "address",
        attrs: &[],
    },
    Schema {
        name: "h1",
        attrs: &[],
    },
    Schema {
        name: "h2",
        attrs: &[],
    },
    Schema {
        name: "h3",
        attrs: &[],
    },
    Schema {
        name: "h4",
        attrs: &[],
    },
    Schema {
        name: "h5",
        attrs: &[],
    },
    Schema {
        name: "h6",
        attrs: &[],
    },
    Schema {
        name: "img",
        attrs: &[("alt", None)],
    },
    Schema {
        name: "img",
        attrs: &[],
    },
    Schema {
        name: "ins",
        attrs: &[],
    },
    Schema {
        name: "menu",
        attrs: &[],
    },
    Schema {
        name: "ol",
        attrs: &[],
    },
    Schema {
        name: "ul",
        attrs: &[],
    },
    Schema {
        name: "li",
        attrs: &[],
    },
    Schema {
        name: "main",
        attrs: &[],
    },
    Schema {
        name: "mark",
        attrs: &[],
    },
    Schema {
        name: "math",
        attrs: &[],
    },
    Schema {
        name: "meter",
        attrs: &[],
    },
    Schema {
        name: "nav",
        attrs: &[],
    },
    Schema {
        name: "p",
        attrs: &[],
    },
    Schema {
        name: "img",
        attrs: &[("alt", Some(""))],
    },
    Schema {
        name: "progress",
        attrs: &[],
    },
    Schema {
        name: "section",
        attrs: &[("aria-label", None)],
    },
    Schema {
        name: "section",
        attrs: &[("aria-labelledby", None)],
    },
    Schema {
        name: "tbody",
        attrs: &[],
    },
    Schema {
        name: "tfoot",
        attrs: &[],
    },
    Schema {
        name: "thead",
        attrs: &[],
    },
    Schema {
        name: "hr",
        attrs: &[],
    },
    Schema {
        name: "output",
        attrs: &[],
    },
    Schema {
        name: "strong",
        attrs: &[],
    },
    Schema {
        name: "sub",
        attrs: &[],
    },
    Schema {
        name: "sup",
        attrs: &[],
    },
    Schema {
        name: "table",
        attrs: &[],
    },
    Schema {
        name: "dfn",
        attrs: &[],
    },
    Schema {
        name: "dt",
        attrs: &[],
    },
    Schema {
        name: "time",
        attrs: &[],
    },
    // ax-non-interactive additions.
    Schema {
        name: "abbr",
        attrs: &[],
    },
    Schema {
        name: "dl",
        attrs: &[],
    },
    Schema {
        name: "dir",
        attrs: &[],
    },
    Schema {
        name: "figcaption",
        attrs: &[],
    },
    Schema {
        name: "form",
        attrs: &[],
    },
    Schema {
        name: "img",
        attrs: &[("usemap", None)],
    },
    Schema {
        name: "label",
        attrs: &[],
    },
    Schema {
        name: "legend",
        attrs: &[],
    },
    Schema {
        name: "br",
        attrs: &[],
    },
    Schema {
        name: "marquee",
        attrs: &[],
    },
    Schema {
        name: "pre",
        attrs: &[],
    },
    Schema {
        name: "tr",
        attrs: &[],
    },
    Schema {
        name: "ruby",
        attrs: &[],
    },
];
