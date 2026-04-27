//! JSDoc heuristics over instance-script source text.
//!
//! These scanners answer two questions about a JS Svelte component:
//!
//!   - Is there a `@typedef {Object} <Name>` block at top-level, and
//!     what is its name? Used to wire `<Name>` (or `Props`) into the
//!     default export's `Awaited<ReturnType<typeof $$render>>['props']`
//!     projection.
//!   - For the `Props` typedef specifically: which `@property` keys
//!     does it declare? Used by emit to decide whether to synthesise
//!     `$$ComponentProps` from a `$props()` destructure (only when no
//!     user `Props` typedef exists) versus keep the user's typedef.
//!
//! JSDoc lives inside JS comments, so there is no AST node for these
//! tags — every JSDoc tool (TypeScript itself, jsdoc.app, eslint-jsdoc)
//! string-scans the comment payload. We do the same on the raw script
//! text. The `@typedef` / `@property` markers are distinctive enough
//! that false positives from a documented string literal are
//! vanishingly rare.
//!
//! Lives in analyze (not emit) so emit reads pre-computed answers
//! rather than re-scanning the script per-call. Pairs with `PropsInfo`
//! to drive the synth-vs-user-Props decision in props_emit.

use crate::PropsInfo;

/// Scan a JS script for a JSDoc `@typedef {Object} Name` block and
/// return the type name. Prefers a typedef named "Props" when
/// multiple are present — the Svelte-4 / JS-Svelte convention that
/// `/** @type {Props} */ let {...} = $props()` reads from. Falls back
/// to the first typedef when no "Props" is present.
pub fn scan_jsdoc_typedef_name(script: &str) -> Option<String> {
    let mut first: Option<String> = None;
    let mut rest = script;
    while let Some(pos) = rest.find("@typedef") {
        let after = &rest[pos + "@typedef".len()..];
        let after = after.trim_start();
        let after = if let Some(stripped) = after.strip_prefix('{') {
            let mut depth = 1u32;
            let mut end = 0usize;
            for (i, c) in stripped.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            &stripped[end..]
        } else {
            after
        };
        let after = after.trim_start();
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if !name.is_empty() && !name.chars().next().is_some_and(|c| c.is_ascii_digit()) {
            if name == "Props" {
                return Some(name);
            }
            if first.is_none() {
                first = Some(name);
            }
        }
        rest = &rest[pos + "@typedef".len()..];
    }
    first
}

/// Extract the `@property` key names from the user's
/// `@typedef {Object} Props` (or `Props<...>`) block. Used to decide
/// whether to synthesise `$$ComponentProps` from a `$props()`
/// destructure when no user Props typedef is present.
///
/// Returns:
///   - `Some(keys)` when a Props typedef block was found (possibly
///     empty if there are no `@property` lines).
///   - `None` when no Props typedef is present.
///
/// Walks the block between the `@typedef` start and the next `*/`,
/// collecting the identifier that follows each `@property {...}`
/// (skipping the type-spec inside balanced `{...}` and the optional
/// `[` for optional markers).
pub fn scan_jsdoc_props_typedef_keys(script: &str) -> Option<Vec<String>> {
    let mut rest = script;
    let mut block_start: Option<usize> = None;
    let mut block_end: usize = 0;
    let mut consumed: usize = 0;
    while let Some(pos) = rest.find("@typedef") {
        let abs = consumed + pos;
        let after = &rest[pos + "@typedef".len()..];
        let after = after.trim_start();
        let after = if let Some(stripped) = after.strip_prefix('{') {
            let mut depth = 1u32;
            let mut end = 0usize;
            for (i, c) in stripped.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            &stripped[end..]
        } else {
            after
        };
        let after = after.trim_start();
        let name: String = after
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if name == "Props" {
            block_start = Some(abs);
            block_end = script[abs..]
                .find("*/")
                .map(|e| abs + e)
                .unwrap_or(script.len());
            break;
        }
        consumed = abs + "@typedef".len();
        rest = &script[consumed..];
    }
    let start = block_start?;
    let block = &script[start..block_end];
    let mut keys: Vec<String> = Vec::new();
    let mut cursor = block;
    while let Some(p) = cursor.find("@property") {
        cursor = &cursor[p + "@property".len()..];
        let trimmed = cursor.trim_start();
        let after_type = if let Some(stripped) = trimmed.strip_prefix('{') {
            let mut depth = 1i32;
            let mut end = 0usize;
            for (i, c) in stripped.char_indices() {
                match c {
                    '{' => depth += 1,
                    '}' => {
                        depth -= 1;
                        if depth == 0 {
                            end = i + 1;
                            break;
                        }
                    }
                    _ => {}
                }
            }
            &stripped[end..]
        } else {
            trimmed
        };
        let after_type = after_type.trim_start();
        let rest_name = after_type.strip_prefix('[').unwrap_or(after_type);
        let name: String = rest_name
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '$')
            .collect();
        if !name.is_empty() {
            keys.push(name);
        }
        cursor = after_type;
    }
    Some(keys)
}

/// Decide whether to synthesise `$$ComponentProps` for a JS overlay
/// vs keep the user's Props typedef. Currently: synthesise only when
/// the user has no Props at all.
///
/// Tried matching upstream's "synth when destructure introduces
/// non-bindable keys not in Props" heuristic — empirically regresses
/// because user Props (when present) is a stricter type than our
/// `any`-laden synthesis can reproduce.
pub fn should_synthesise_js_props(props_info: &PropsInfo, script: &str) -> bool {
    if props_info.destructures.is_empty() {
        return false;
    }
    scan_jsdoc_props_typedef_keys(script).is_none()
}
