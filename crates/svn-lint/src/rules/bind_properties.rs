//! The compiler's binding table (`phases/bindings.js`): which
//! `bind:` names exist and which elements each one is valid on.

/// One entry of the table. `valid_elements` restricts the binding to
/// those tags; `invalid_elements` forbids it on those tags; both empty
/// means any element.
pub(crate) struct BindingProperty {
    pub name: &'static str,
    pub valid_elements: &'static [&'static str],
    pub invalid_elements: &'static [&'static str],
}

const MEDIA: &[&str] = &["audio", "video"];
const NOT_WINDOW_OR_DOCUMENT: &[&str] = &["svelte:window", "svelte:document"];
const WINDOW: &[&str] = &["svelte:window"];
const DOCUMENT: &[&str] = &["svelte:document"];
const INPUT: &[&str] = &["input"];

macro_rules! b {
    ($name:literal) => {
        BindingProperty {
            name: $name,
            valid_elements: &[],
            invalid_elements: &[],
        }
    };
    ($name:literal, valid = $valid:expr) => {
        BindingProperty {
            name: $name,
            valid_elements: $valid,
            invalid_elements: &[],
        }
    };
    ($name:literal, invalid = $invalid:expr) => {
        BindingProperty {
            name: $name,
            valid_elements: &[],
            invalid_elements: $invalid,
        }
    };
}

pub(crate) const BINDING_PROPERTIES: &[BindingProperty] = &[
    b!("currentTime", valid = MEDIA),
    b!("duration", valid = MEDIA),
    b!("focused"),
    b!("paused", valid = MEDIA),
    b!("buffered", valid = MEDIA),
    b!("seekable", valid = MEDIA),
    b!("played", valid = MEDIA),
    b!("volume", valid = MEDIA),
    b!("muted", valid = MEDIA),
    b!("playbackRate", valid = MEDIA),
    b!("seeking", valid = MEDIA),
    b!("ended", valid = MEDIA),
    b!("readyState", valid = MEDIA),
    b!("videoHeight", valid = &["video"]),
    b!("videoWidth", valid = &["video"]),
    b!("naturalWidth", valid = &["img"]),
    b!("naturalHeight", valid = &["img"]),
    b!("activeElement", valid = DOCUMENT),
    b!("fullscreenElement", valid = DOCUMENT),
    b!("pointerLockElement", valid = DOCUMENT),
    b!("visibilityState", valid = DOCUMENT),
    b!("innerWidth", valid = WINDOW),
    b!("innerHeight", valid = WINDOW),
    b!("outerWidth", valid = WINDOW),
    b!("outerHeight", valid = WINDOW),
    b!("scrollX", valid = WINDOW),
    b!("scrollY", valid = WINDOW),
    b!("online", valid = WINDOW),
    b!("devicePixelRatio", valid = WINDOW),
    b!("clientWidth", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("clientHeight", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("offsetWidth", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("offsetHeight", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("contentRect", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("contentBoxSize", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("borderBoxSize", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!(
        "devicePixelContentBoxSize",
        invalid = NOT_WINDOW_OR_DOCUMENT
    ),
    b!("indeterminate", valid = INPUT),
    b!("checked", valid = INPUT),
    b!("group", valid = INPUT),
    b!("this"),
    b!("innerText", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("innerHTML", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("textContent", invalid = NOT_WINDOW_OR_DOCUMENT),
    b!("open", valid = &["details"]),
    b!("value", valid = &["input", "textarea", "select"]),
    b!("files", valid = INPUT),
];

pub(crate) fn lookup(name: &str) -> Option<&'static BindingProperty> {
    BINDING_PROPERTIES.iter().find(|p| p.name == name)
}

/// Is `property` allowed on `element` per its valid/invalid lists?
pub(crate) fn allowed_on(property: &BindingProperty, element: &str) -> bool {
    if !property.valid_elements.is_empty() {
        return property.valid_elements.contains(&element);
    }
    !property.invalid_elements.contains(&element)
}
