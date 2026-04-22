//! HTML5 tree-placement validation, transcribed verbatim from
//! `sveltejs/svelte` `packages/svelte/src/html-tree-validation.js`.
//!
//! Keep the public surface byte-parity with upstream:
//! - `is_tag_valid_with_parent(child, parent) -> Option<String>`
//! - `is_tag_valid_with_ancestor(child, ancestors) -> Option<String>`
//! - `closing_tag_omitted(current, next) -> bool`
//!
//! When refreshing (`/update-lint`), diff the source against the
//! vendored clone and port any table changes across.

/// Entry in the disallowed-children table.
enum Entry {
    /// Child is disallowed as a DIRECT child only.
    Direct(&'static [&'static str]),
    /// Child is disallowed as any DESCENDANT until `reset_by` ancestor.
    Descendant {
        descendants: &'static [&'static str],
        reset_by: &'static [&'static str],
    },
    /// Parent permits ONLY these direct children. Everything else is disallowed.
    Only { only: &'static [&'static str] },
}

/// Upstream `disallowed_children` table. Used by
/// `is_tag_valid_with_parent` / `_ancestor`.
fn disallowed_children(tag: &str) -> Option<Entry> {
    // Items from autoclosing_children that have `descendant`:
    match tag {
        "li" => Some(Entry::Direct(&["li"])),
        "dt" => Some(Entry::Descendant {
            descendants: &["dt", "dd"],
            reset_by: &["dl"],
        }),
        "dd" => Some(Entry::Descendant {
            descendants: &["dt", "dd"],
            reset_by: &["dl"],
        }),
        "p" => Some(Entry::Descendant {
            descendants: &[
                "address",
                "article",
                "aside",
                "blockquote",
                "div",
                "dl",
                "fieldset",
                "footer",
                "form",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "header",
                "hgroup",
                "hr",
                "main",
                "menu",
                "nav",
                "ol",
                "p",
                "pre",
                "section",
                "table",
                "ul",
            ],
            reset_by: &[],
        }),
        "rt" => Some(Entry::Descendant {
            descendants: &["rt", "rp"],
            reset_by: &[],
        }),
        "rp" => Some(Entry::Descendant {
            descendants: &["rt", "rp"],
            reset_by: &[],
        }),
        "optgroup" => Some(Entry::Descendant {
            descendants: &["optgroup"],
            reset_by: &[],
        }),
        "option" => Some(Entry::Descendant {
            descendants: &["option", "optgroup"],
            reset_by: &[],
        }),
        "thead" => Some(Entry::Only {
            only: &["tr", "style", "script", "template"],
        }),
        "tbody" => Some(Entry::Only {
            only: &["tr", "style", "script", "template"],
        }),
        "tfoot" => Some(Entry::Only {
            only: &["tr", "style", "script", "template"],
        }),
        "tr" => Some(Entry::Only {
            only: &["th", "td", "style", "script", "template"],
        }),
        "td" => Some(Entry::Direct(&["td", "th", "tr"])),
        "th" => Some(Entry::Direct(&["td", "th", "tr"])),

        // ... extras in disallowed_children beyond autoclosing_children:
        "form" => Some(Entry::Descendant {
            descendants: &["form"],
            reset_by: &[],
        }),
        "a" => Some(Entry::Descendant {
            descendants: &["a"],
            reset_by: &[],
        }),
        "button" => Some(Entry::Descendant {
            descendants: &["button"],
            reset_by: &[],
        }),
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => Some(Entry::Descendant {
            descendants: &["h1", "h2", "h3", "h4", "h5", "h6"],
            reset_by: &[],
        }),
        "colgroup" => Some(Entry::Only {
            only: &["col", "template"],
        }),
        "table" => Some(Entry::Only {
            only: &[
                "caption", "colgroup", "tbody", "thead", "tfoot", "style", "script", "template",
            ],
        }),
        "head" => Some(Entry::Only {
            only: &[
                "base", "basefont", "bgsound", "link", "meta", "title", "noscript", "noframes",
                "style", "script", "template",
            ],
        }),
        "html" => Some(Entry::Only {
            only: &["head", "body", "frameset"],
        }),
        "frameset" => Some(Entry::Only { only: &["frame"] }),
        "#document" => Some(Entry::Only { only: &["html"] }),
        _ => None,
    }
}

/// Autoclosing subset of `disallowed_children` — used by
/// `closing_tag_omitted`.
fn autoclosing(tag: &str) -> Option<Entry> {
    match tag {
        "li" => Some(Entry::Direct(&["li"])),
        "dt" => Some(Entry::Descendant {
            descendants: &["dt", "dd"],
            reset_by: &["dl"],
        }),
        "dd" => Some(Entry::Descendant {
            descendants: &["dt", "dd"],
            reset_by: &["dl"],
        }),
        "p" => Some(Entry::Descendant {
            descendants: &[
                "address",
                "article",
                "aside",
                "blockquote",
                "div",
                "dl",
                "fieldset",
                "footer",
                "form",
                "h1",
                "h2",
                "h3",
                "h4",
                "h5",
                "h6",
                "header",
                "hgroup",
                "hr",
                "main",
                "menu",
                "nav",
                "ol",
                "p",
                "pre",
                "section",
                "table",
                "ul",
            ],
            reset_by: &[],
        }),
        "rt" => Some(Entry::Descendant {
            descendants: &["rt", "rp"],
            reset_by: &[],
        }),
        "rp" => Some(Entry::Descendant {
            descendants: &["rt", "rp"],
            reset_by: &[],
        }),
        "optgroup" => Some(Entry::Descendant {
            descendants: &["optgroup"],
            reset_by: &[],
        }),
        "option" => Some(Entry::Descendant {
            descendants: &["option", "optgroup"],
            reset_by: &[],
        }),
        "thead" => Some(Entry::Direct(&["tbody", "tfoot"])),
        "tbody" => Some(Entry::Direct(&["tbody", "tfoot"])),
        "tfoot" => Some(Entry::Direct(&["tbody"])),
        "tr" => Some(Entry::Direct(&["tr", "tbody"])),
        "td" => Some(Entry::Direct(&["td", "th", "tr"])),
        "th" => Some(Entry::Direct(&["td", "th", "tr"])),
        _ => None,
    }
}

/// Returns true if `current` is implicitly closed by `next` (or at
/// EOF when `next` is None).
pub fn closing_tag_omitted(current: &str, next: Option<&str>) -> bool {
    let Some(entry) = autoclosing(current) else {
        return false;
    };
    match (entry, next) {
        (Entry::Direct(d), Some(n)) => d.contains(&n),
        (Entry::Descendant { descendants, .. }, Some(n)) => descendants.contains(&n),
        (_, None) => true,
        _ => false,
    }
}

/// Returns Some(error_message) if `child_tag` can't be a direct
/// child of `parent_tag`. Otherwise None.
pub fn is_tag_valid_with_parent(child_tag: &str, parent_tag: &str) -> Option<String> {
    if child_tag.contains('-') || parent_tag.contains('-') {
        return None;
    }
    if parent_tag == "template" {
        return None;
    }
    let child = format!("`<{child_tag}>`");
    let parent = format!("`<{parent_tag}>`");

    if let Some(entry) = disallowed_children(parent_tag) {
        match entry {
            Entry::Direct(direct) => {
                if direct.contains(&child_tag) {
                    return Some(format!("{child} cannot be a direct child of {parent}"));
                }
            }
            Entry::Descendant { descendants, .. } => {
                if descendants.contains(&child_tag) {
                    return Some(format!("{child} cannot be a child of {parent}"));
                }
            }
            Entry::Only { only } => {
                if !only.contains(&child_tag) {
                    let list = only
                        .iter()
                        .map(|t| format!("`<{t}>`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    return Some(format!(
                        "{child} cannot be a child of {parent}. `<{parent_tag}>` only allows these children: {list}"
                    ));
                }
                return None;
            }
        }
    }

    match child_tag {
        "body" | "caption" | "col" | "colgroup" | "frameset" | "frame" | "head" | "html" => {
            Some(format!("{child} cannot be a child of {parent}"))
        }
        "thead" | "tbody" | "tfoot" => Some(format!(
            "{child} must be the child of a `<table>`, not a {parent}"
        )),
        "td" | "th" => Some(format!(
            "{child} must be the child of a `<tr>`, not a {parent}"
        )),
        "tr" => Some(format!(
            "`<tr>` must be the child of a `<thead>`, `<tbody>`, or `<tfoot>`, not a {parent}"
        )),
        _ => None,
    }
}

/// Returns Some(error_message) if `child_tag` violates a
/// descendant-level restriction rooted at `ancestors[last]`.
pub fn is_tag_valid_with_ancestor(child_tag: &str, ancestors: &[&str]) -> Option<String> {
    if child_tag.contains('-') {
        return None;
    }
    let ancestor_tag = *ancestors.last()?;
    let entry = disallowed_children(ancestor_tag)?;

    if let Entry::Descendant {
        descendants,
        reset_by,
    } = &entry
        && !reset_by.is_empty()
    {
        // Walk from second-to-last upward, stop at reset_by.
        for ancestor in ancestors.iter().rev().skip(1) {
            if ancestor.contains('-') {
                return None;
            }
            if reset_by.contains(ancestor) {
                return None;
            }
        }
        if descendants.contains(&child_tag) {
            let child = format!("`<{child_tag}>`");
            let ancestor = format!("`<{ancestor_tag}>`");
            return Some(format!("{child} cannot be a descendant of {ancestor}"));
        }
    } else if let Entry::Descendant { descendants, .. } = &entry
        && descendants.contains(&child_tag)
    {
        let child = format!("`<{child_tag}>`");
        let ancestor = format!("`<{ancestor_tag}>`");
        return Some(format!("{child} cannot be a descendant of {ancestor}"));
    }

    None
}
