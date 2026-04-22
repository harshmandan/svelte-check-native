//! Static a11y tables transcribed from upstream
//! `packages/svelte/src/compiler/phases/2-analyze/visitors/shared/a11y/constants.js`.
//!
//! Purely data — no `aria-query` / `axobject-query` dependency yet
//! (those land with Phase E.3). Rules that don't need the full ARIA
//! role tables consult the tables below directly.

/// ARIA attributes (without the `aria-` prefix). Mirrors upstream's
/// `aria_attributes` array.
pub const ARIA_ATTRIBUTES: &[&str] = &[
    "activedescendant",
    "atomic",
    "autocomplete",
    "busy",
    "checked",
    "colcount",
    "colindex",
    "colspan",
    "controls",
    "current",
    "describedby",
    "description",
    "details",
    "disabled",
    "dropeffect",
    "errormessage",
    "expanded",
    "flowto",
    "grabbed",
    "haspopup",
    "hidden",
    "invalid",
    "keyshortcuts",
    "label",
    "labelledby",
    "level",
    "live",
    "modal",
    "multiline",
    "multiselectable",
    "orientation",
    "owns",
    "placeholder",
    "posinset",
    "pressed",
    "readonly",
    "relevant",
    "required",
    "roledescription",
    "rowcount",
    "rowindex",
    "rowspan",
    "selected",
    "setsize",
    "sort",
    "valuemax",
    "valuemin",
    "valuenow",
    "valuetext",
];

/// Required attributes per element — at least one of the listed
/// names must be present.
pub fn a11y_required_attributes(name: &str) -> Option<&'static [&'static str]> {
    Some(match name {
        "a" => &["href"],
        "area" => &["alt", "aria-label", "aria-labelledby"],
        "html" => &["lang"],
        "iframe" => &["title"],
        "img" => &["alt"],
        "object" => &["title", "aria-label", "aria-labelledby"],
        _ => return None,
    })
}

/// `<blink>` / `<marquee>`.
pub const A11Y_DISTRACTING_ELEMENTS: &[&str] = &["blink", "marquee"];

/// Heading tags that fire `a11y_missing_content` if empty.
/// `<a>` and `<button>` are handled via a separate path that also
/// checks aria-label.
pub const A11Y_REQUIRED_CONTENT: &[&str] = &["h1", "h2", "h3", "h4", "h5", "h6"];

/// Form controls that can be `<label>` targets.
pub const A11Y_LABELABLE: &[&str] = &[
    "button", "input", "keygen", "meter", "output", "progress", "select", "textarea",
];

/// Event names that count as "interactive" on an element.
///
/// Tracks upstream `packages/svelte/.../a11y/constants.js`'s
/// `a11y_interactive_handlers`. Newer svelte (>= 5.56-ish) extended
/// the list with pointer/touch events; older svelte (<= 5.55) stops
/// at mouse. We follow the newer list because our validator fixtures
/// come from the `.svelte-upstream` submodule which pins main.
/// Result: on workspaces whose `node_modules/svelte` predates the
/// extension, our native pass fires on pointer/touch events upstream
/// didn't — a known bounded over-fire that goes away once the
/// workspace upgrades its compiler.
pub const A11Y_INTERACTIVE_HANDLERS: &[&str] = &[
    "keypress",
    "keydown",
    "keyup",
    "click",
    "contextmenu",
    "dblclick",
    "drag",
    "dragend",
    "dragenter",
    "dragexit",
    "dragleave",
    "dragover",
    "dragstart",
    "drop",
    "mousedown",
    "mouseenter",
    "mouseleave",
    "mousemove",
    "mouseout",
    "mouseover",
    "mouseup",
    "pointerdown",
    "pointerup",
    "pointermove",
    "pointerenter",
    "pointerleave",
    "pointerover",
    "pointerout",
    "pointercancel",
    "touchstart",
    "touchend",
    "touchmove",
    "touchcancel",
];

/// Subset of `A11Y_INTERACTIVE_HANDLERS` that trigger
/// `a11y_recommended_interactive_handlers` (used inside
/// `a11y_click_events_have_key_events` / friends).
pub const A11Y_RECOMMENDED_INTERACTIVE_HANDLERS: &[&str] =
    &["click", "mousedown", "mouseup", "keypress", "keydown", "keyup"];

/// `<header>` / `<footer>` — role depends on enclosing `<section>` /
/// `<article>`.
pub fn a11y_nested_implicit_semantics(name: &str) -> Option<&'static str> {
    match name {
        "header" => Some("banner"),
        "footer" => Some("contentinfo"),
        _ => None,
    }
}

/// Implicit role for an element (matching upstream
/// `a11y_implicit_semantics` map).
pub fn a11y_implicit_semantics(name: &str) -> Option<&'static str> {
    Some(match name {
        "a" => "link",
        "area" => "link",
        "article" => "article",
        "aside" => "complementary",
        "body" => "document",
        "button" => "button",
        "datalist" => "listbox",
        "dd" => "definition",
        "dfn" => "term",
        "dialog" => "dialog",
        "details" => "group",
        "dt" => "term",
        "fieldset" => "group",
        "figure" => "figure",
        "form" => "form",
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => "heading",
        "hr" => "separator",
        "img" => "img",
        "li" => "listitem",
        "link" => "link",
        "main" => "main",
        "menu" => "list",
        "meter" => "progressbar",
        "nav" => "navigation",
        "ol" => "list",
        "option" => "option",
        "optgroup" => "group",
        "output" => "status",
        "progress" => "progressbar",
        "section" => "region",
        "summary" => "button",
        "table" => "table",
        "tbody" => "rowgroup",
        "textarea" => "textbox",
        "tfoot" => "rowgroup",
        "thead" => "rowgroup",
        "tr" => "row",
        "ul" => "list",
        _ => return None,
    })
}

/// `<menuitem type="X">` → role.
pub fn menuitem_type_to_implicit_role(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "command" => "menuitem",
        "checkbox" => "menuitemcheckbox",
        "radio" => "menuitemradio",
        _ => return None,
    })
}

/// `<input type="X">` → role.
pub fn input_type_to_implicit_role(ty: &str) -> Option<&'static str> {
    Some(match ty {
        "button" | "image" | "reset" | "submit" => "button",
        "checkbox" => "checkbox",
        "radio" => "radio",
        "range" => "slider",
        "number" => "spinbutton",
        "email" | "tel" | "text" | "url" => "textbox",
        "search" => "searchbox",
        _ => return None,
    })
}

/// Exceptions to the default "non-interactive element can't have an
/// interactive role" rule — common conventions we allow.
pub fn a11y_non_interactive_element_to_interactive_role_exceptions(
    name: &str,
) -> Option<&'static [&'static str]> {
    Some(match name {
        "ul" | "ol" | "menu" => &[
            "listbox", "menu", "menubar", "radiogroup", "tablist", "tree", "treegrid",
        ],
        "li" => &["menuitem", "option", "row", "tab", "treeitem"],
        "table" => &["grid"],
        "td" => &["gridcell"],
        "fieldset" => &["radiogroup", "presentation"],
        _ => return None,
    })
}

/// `<input type="X">` — candidate `combobox` implicit role (the
/// actual rune depends on `list` attribute presence).
pub const COMBOBOX_IF_LIST: &[&str] = &["email", "search", "tel", "text", "url"];

pub const ADDRESS_TYPE_TOKENS: &[&str] = &["shipping", "billing"];
pub const CONTACT_TYPE_TOKENS: &[&str] = &["home", "work", "mobile", "fax", "pager"];

pub const AUTOFILL_FIELD_NAME_TOKENS: &[&str] = &[
    "",
    "on",
    "off",
    "name",
    "honorific-prefix",
    "given-name",
    "additional-name",
    "family-name",
    "honorific-suffix",
    "nickname",
    "username",
    "new-password",
    "current-password",
    "one-time-code",
    "organization-title",
    "organization",
    "street-address",
    "address-line1",
    "address-line2",
    "address-line3",
    "address-level4",
    "address-level3",
    "address-level2",
    "address-level1",
    "country",
    "country-name",
    "postal-code",
    "cc-name",
    "cc-given-name",
    "cc-additional-name",
    "cc-family-name",
    "cc-number",
    "cc-exp",
    "cc-exp-month",
    "cc-exp-year",
    "cc-csc",
    "cc-type",
    "transaction-currency",
    "transaction-amount",
    "language",
    "bday",
    "bday-day",
    "bday-month",
    "bday-year",
    "sex",
    "url",
    "photo",
];

pub const AUTOFILL_CONTACT_FIELD_NAME_TOKENS: &[&str] = &[
    "tel",
    "tel-country-code",
    "tel-national",
    "tel-area-code",
    "tel-local",
    "tel-local-prefix",
    "tel-local-suffix",
    "tel-extension",
    "email",
    "impp",
];

/// Elements that never render visible content; aria-*/role attrs on
/// them fire `a11y_aria_attributes` / `a11y_misplaced_role`.
pub const INVISIBLE_ELEMENTS: &[&str] = &["meta", "html", "script", "style"];

/// Shape of an ARIA property's value-type constraint. Mirrors
/// upstream `aria-query`'s `ariaPropsMap.js` (48 entries); vendored
/// here so our linter stays dep-free.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AriaType {
    Boolean,
    BooleanUndefined,
    Integer,
    Number,
    String,
    Id,
    IdList,
    /// Single-token — must match one of the allowed `values`.
    Token,
    /// Space-separated list of tokens, each in `values`.
    TokenList,
    /// `"true"` | `"false"` | `"mixed"`.
    Tristate,
}

pub struct AriaPropDef {
    pub ty: AriaType,
    pub values: &'static [&'static str],
}

/// Return the ARIA property definition for `name` (including the
/// leading `aria-`). `None` for unknown properties.
pub fn aria_prop(name: &str) -> Option<&'static AriaPropDef> {
    for (k, v) in ARIA_PROPS {
        if *k == name {
            return Some(v);
        }
    }
    None
}

const NONE: &[&str] = &[];

/// All valid ARIA role names. Transcribed from `aria-query`'s
/// `rolesMap` (147 entries including `doc-*` and `graphics-*`).
pub const ARIA_ROLES: &[&str] = &[
    "command",
    "composite",
    "input",
    "landmark",
    "range",
    "roletype",
    "section",
    "sectionhead",
    "select",
    "structure",
    "widget",
    "window",
    "alert",
    "alertdialog",
    "application",
    "article",
    "banner",
    "blockquote",
    "button",
    "caption",
    "cell",
    "checkbox",
    "code",
    "columnheader",
    "combobox",
    "complementary",
    "contentinfo",
    "definition",
    "deletion",
    "dialog",
    "directory",
    "document",
    "emphasis",
    "feed",
    "figure",
    "form",
    "generic",
    "grid",
    "gridcell",
    "group",
    "heading",
    "img",
    "insertion",
    "link",
    "list",
    "listbox",
    "listitem",
    "log",
    "main",
    "mark",
    "marquee",
    "math",
    "menu",
    "menubar",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "meter",
    "navigation",
    "none",
    "note",
    "option",
    "paragraph",
    "presentation",
    "progressbar",
    "radio",
    "radiogroup",
    "region",
    "row",
    "rowgroup",
    "rowheader",
    "scrollbar",
    "search",
    "searchbox",
    "separator",
    "slider",
    "spinbutton",
    "status",
    "strong",
    "subscript",
    "superscript",
    "switch",
    "tab",
    "table",
    "tablist",
    "tabpanel",
    "term",
    "textbox",
    "time",
    "timer",
    "toolbar",
    "tooltip",
    "tree",
    "treegrid",
    "treeitem",
    "doc-abstract",
    "doc-acknowledgments",
    "doc-afterword",
    "doc-appendix",
    "doc-backlink",
    "doc-biblioentry",
    "doc-bibliography",
    "doc-biblioref",
    "doc-chapter",
    "doc-colophon",
    "doc-conclusion",
    "doc-cover",
    "doc-credit",
    "doc-credits",
    "doc-dedication",
    "doc-endnote",
    "doc-endnotes",
    "doc-epigraph",
    "doc-epilogue",
    "doc-errata",
    "doc-example",
    "doc-footnote",
    "doc-foreword",
    "doc-glossary",
    "doc-glossref",
    "doc-index",
    "doc-introduction",
    "doc-noteref",
    "doc-notice",
    "doc-pagebreak",
    "doc-pagefooter",
    "doc-pageheader",
    "doc-pagelist",
    "doc-part",
    "doc-preface",
    "doc-prologue",
    "doc-pullquote",
    "doc-qna",
    "doc-subtitle",
    "doc-tip",
    "doc-toc",
    "graphics-document",
    "graphics-object",
    "graphics-symbol",
];

/// Abstract ARIA roles — role names that shouldn't appear on elements
/// directly.
pub const ABSTRACT_ROLES: &[&str] = &[
    "command",
    "composite",
    "input",
    "landmark",
    "range",
    "roletype",
    "section",
    "sectionhead",
    "select",
    "structure",
    "widget",
    "window",
];

/// Roles classified as interactive — derived from aria-query's
/// widget/window superClass descent (minus the carve-outs for
/// `toolbar`, `tabpanel`, `generic`, `cell`, and progressbar).
pub const INTERACTIVE_ROLES: &[&str] = &[
    "alertdialog",
    "button",
    "cell",
    "checkbox",
    "columnheader",
    "combobox",
    "dialog",
    "grid",
    "gridcell",
    "link",
    "listbox",
    "menu",
    "menubar",
    "menuitem",
    "menuitemcheckbox",
    "menuitemradio",
    "option",
    "radio",
    "radiogroup",
    "row",
    "rowheader",
    "scrollbar",
    "searchbox",
    "slider",
    "spinbutton",
    "switch",
    "tab",
    "tablist",
    "tabpanel",
    "textbox",
    "toolbar",
    "tree",
    "treegrid",
    "treeitem",
    "doc-backlink",
    "doc-biblioref",
    "doc-glossref",
    "doc-noteref",
];

/// Roles classified as non-interactive — complement of
/// `INTERACTIVE_ROLES` within the non-abstract role space.
pub const NON_INTERACTIVE_ROLES: &[&str] = &[
    "alert",
    "application",
    "article",
    "banner",
    "blockquote",
    "caption",
    "code",
    "complementary",
    "contentinfo",
    "definition",
    "deletion",
    "directory",
    "document",
    "emphasis",
    "feed",
    "figure",
    "form",
    "group",
    "heading",
    "img",
    "insertion",
    "list",
    "listitem",
    "log",
    "main",
    "mark",
    "marquee",
    "math",
    "meter",
    "navigation",
    "none",
    "note",
    "paragraph",
    "presentation",
    "region",
    "rowgroup",
    "search",
    "separator",
    "status",
    "strong",
    "subscript",
    "superscript",
    "table",
    "term",
    "time",
    "timer",
    "tooltip",
    "progressbar",
];

pub const PRESENTATION_ROLES: &[&str] = &["presentation", "none"];

pub static ARIA_PROPS: &[(&str, &AriaPropDef)] = &[
    ("aria-activedescendant", &AriaPropDef { ty: AriaType::Id, values: NONE }),
    ("aria-atomic", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-autocomplete", &AriaPropDef { ty: AriaType::Token, values: &["inline", "list", "both", "none"] }),
    ("aria-braillelabel", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-brailleroledescription", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-busy", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-checked", &AriaPropDef { ty: AriaType::Tristate, values: NONE }),
    ("aria-colcount", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-colindex", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-colspan", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-controls", &AriaPropDef { ty: AriaType::IdList, values: NONE }),
    ("aria-current", &AriaPropDef { ty: AriaType::Token, values: &["page", "step", "location", "date", "time", "true", "false"] }),
    ("aria-describedby", &AriaPropDef { ty: AriaType::IdList, values: NONE }),
    ("aria-description", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-details", &AriaPropDef { ty: AriaType::Id, values: NONE }),
    ("aria-disabled", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-dropeffect", &AriaPropDef { ty: AriaType::TokenList, values: &["copy", "execute", "link", "move", "none", "popup"] }),
    ("aria-errormessage", &AriaPropDef { ty: AriaType::Id, values: NONE }),
    ("aria-expanded", &AriaPropDef { ty: AriaType::BooleanUndefined, values: NONE }),
    ("aria-flowto", &AriaPropDef { ty: AriaType::IdList, values: NONE }),
    ("aria-grabbed", &AriaPropDef { ty: AriaType::BooleanUndefined, values: NONE }),
    ("aria-haspopup", &AriaPropDef { ty: AriaType::Token, values: &["false", "true", "menu", "listbox", "tree", "grid", "dialog"] }),
    ("aria-hidden", &AriaPropDef { ty: AriaType::BooleanUndefined, values: NONE }),
    ("aria-invalid", &AriaPropDef { ty: AriaType::Token, values: &["grammar", "false", "spelling", "true"] }),
    ("aria-keyshortcuts", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-label", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-labelledby", &AriaPropDef { ty: AriaType::IdList, values: NONE }),
    ("aria-level", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-live", &AriaPropDef { ty: AriaType::Token, values: &["assertive", "off", "polite"] }),
    ("aria-modal", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-multiline", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-multiselectable", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-orientation", &AriaPropDef { ty: AriaType::Token, values: &["vertical", "undefined", "horizontal"] }),
    ("aria-owns", &AriaPropDef { ty: AriaType::IdList, values: NONE }),
    ("aria-placeholder", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-posinset", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-pressed", &AriaPropDef { ty: AriaType::Tristate, values: NONE }),
    ("aria-readonly", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-relevant", &AriaPropDef { ty: AriaType::TokenList, values: &["additions", "all", "removals", "text"] }),
    ("aria-required", &AriaPropDef { ty: AriaType::Boolean, values: NONE }),
    ("aria-roledescription", &AriaPropDef { ty: AriaType::String, values: NONE }),
    ("aria-rowcount", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-rowindex", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-rowspan", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-selected", &AriaPropDef { ty: AriaType::BooleanUndefined, values: NONE }),
    ("aria-setsize", &AriaPropDef { ty: AriaType::Integer, values: NONE }),
    ("aria-sort", &AriaPropDef { ty: AriaType::Token, values: &["ascending", "descending", "none", "other"] }),
    ("aria-valuemax", &AriaPropDef { ty: AriaType::Number, values: NONE }),
    ("aria-valuemin", &AriaPropDef { ty: AriaType::Number, values: NONE }),
    ("aria-valuenow", &AriaPropDef { ty: AriaType::Number, values: NONE }),
    ("aria-valuetext", &AriaPropDef { ty: AriaType::String, values: NONE }),
];
