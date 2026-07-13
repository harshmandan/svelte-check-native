//! Small, dependency-free helpers used across the emit crate:
//! line/byte position math, render-function naming, and generic-args
//! extraction. Pulled out of `lib.rs` so the main file isn't forced to
//! carry a tail of unrelated utilities.

use std::path::Path;

use smol_str::SmolStr;
use svn_parser::Document;

/// 1-based current line of a buffer that has already been written.
pub(crate) fn current_line(s: &str) -> u32 {
    1 + s.bytes().filter(|&b| b == b'\n').count() as u32
}

/// Byte offsets of the start of each line in `s`. Index 0 is the start
/// of line 1; index N is the start of line N+1 (i.e. the position just
/// past the Nth `\n`). A final sentinel equal to `s.len()` is appended
/// so `starts[line_count]` is always valid and equals the end of the
/// last line's content — lets the consumer clamp a past-EOF (line,
/// col) to the end of the buffer without bounds-checking.
pub fn compute_line_starts(s: &str) -> Vec<u32> {
    let mut starts: Vec<u32> = Vec::with_capacity(s.bytes().filter(|&b| b == b'\n').count() + 2);
    starts.push(0);
    for (idx, b) in s.bytes().enumerate() {
        if b == b'\n' {
            starts.push((idx + 1) as u32);
        }
    }
    starts.push(s.len() as u32);
    starts
}

/// 1-based line number at the given byte offset in `source`.
#[inline]
pub(crate) fn source_line_at(source: &str, offset: u32) -> u32 {
    1 + source[..offset as usize]
        .bytes()
        .filter(|&b| b == b'\n')
        .count() as u32
}

/// Count the number of complete lines in `text` (the count of `\n` plus
/// 1 if the text doesn't end with a newline and is non-empty). Used to
/// derive an end-line for line-map entries.
#[inline]
pub(crate) fn count_lines(text: &str) -> u32 {
    let nl = text.bytes().filter(|&b| b == b'\n').count() as u32;
    if text.is_empty() {
        0
    } else if text.ends_with('\n') {
        nl
    } else {
        nl + 1
    }
}

/// Derive a per-file render function name. Hash of the source path's
/// canonical form prevents collisions when multiple components in the
/// same overlay project would otherwise both produce `function $$render()`
/// (TS2393 "Duplicate function implementation").
///
/// The hash is the first 8 hex chars of blake3 — collision-free for any
/// realistic project size.
pub(crate) fn render_function_name(source_path: &Path) -> SmolStr {
    let bytes = source_path.as_os_str().to_string_lossy();
    let hash = blake3::hash(bytes.as_bytes());
    let hex = hash.to_hex();
    let short = &hex.as_str()[..8];
    SmolStr::from(format!("$$render_{short}"))
}

/// Companion-class name for [`render_function_name`]. Used by the
/// class-wrapper emit path (Phase 2 / R1 of `notes/PLAN.md`) to
/// extract body-scoped Props types at module scope via
/// `ReturnType<__svn_Render_<hash><T>['props']>`.
///
/// Matches upstream svelte2tsx's `class __sveltets_Render<T>` shape
/// but with our per-file hash prefix — same reason as `$$render_<hash>`:
/// two components in the same overlay project would collide on a bare
/// `class __svn_Render { … }`.
pub(crate) fn render_class_name(render_fn_name: &str) -> SmolStr {
    let short = render_fn_name
        .strip_prefix("$$render_")
        .unwrap_or(render_fn_name);
    SmolStr::from(format!("__svn_Render_{short}"))
}

/// Extract just the type-parameter NAMES from a Svelte-5 generics
/// attribute value.
///
/// Input is what the user wrote in `<script lang="ts" generics="…">`.
/// We splice that string verbatim into the declaration site of
/// `async function $$render_<hash><…>()` and any class-wrapper
/// declaration — both expect the full parameter syntax (with `extends`
/// constraints and `= default` defaults).
///
/// At instantiation sites we need just the names: `typeof foo<T, U>`
/// not `typeof foo<T extends X, U = Y>`. This helper strips
/// constraints and defaults, preserving comma-separated order.
///
/// Handles:
/// - `T` → `T`
/// - `T extends Item` → `T`
/// - `T extends Item, U` → `T, U`
/// - `T extends Array<U>, U` → `T, U` (bracket depth tracked so the
///   inner `U` doesn't get counted as a separator)
/// - `T = string` → `T`
/// - `const T extends X` / `in T` / `out U` → `T` / `T` / `U`
///   (declaration-only modifiers stripped — see [`strip_tp_modifiers`])
pub(crate) fn generic_arg_names(generics: &str) -> String {
    // Output is a strict subset of `generics` (drops constraints /
    // defaults, keeps names + commas). Pre-size to input length so
    // single-shot push_strs don't trigger any growth realloc.
    let mut out = String::with_capacity(generics.len());
    let mut depth_angle: i32 = 0;
    let mut depth_paren: i32 = 0;
    let mut depth_bracket: i32 = 0;
    let mut current_name: Vec<u8> = Vec::new();
    let mut in_name = true;
    let bytes = generics.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'<' => depth_angle += 1,
            b'>' => depth_angle -= 1,
            b'(' => depth_paren += 1,
            b')' => depth_paren -= 1,
            b'[' => depth_bracket += 1,
            b']' => depth_bracket -= 1,
            _ => {}
        }
        let at_depth_zero = depth_angle == 0 && depth_paren == 0 && depth_bracket == 0;
        if at_depth_zero && b == b',' {
            let s = std::str::from_utf8(&current_name).unwrap_or("");
            let trimmed = strip_tp_modifiers(s);
            if !trimmed.is_empty() {
                if !out.is_empty() {
                    out.push_str(", ");
                }
                out.push_str(trimmed);
            }
            current_name.clear();
            in_name = true;
            i += 1;
            continue;
        }
        if in_name && at_depth_zero {
            if b.is_ascii_whitespace() {
                if !current_name.is_empty() {
                    let mut j = i;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if generics[j..].starts_with("extends") {
                        in_name = false;
                    }
                }
                current_name.push(b);
                i += 1;
                continue;
            }
            if b == b'=' {
                in_name = false;
                i += 1;
                continue;
            }
            current_name.push(b);
        }
        i += 1;
    }
    let s = std::str::from_utf8(&current_name).unwrap_or("");
    let trimmed = strip_tp_modifiers(s);
    if !trimmed.is_empty() {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(trimmed);
    }
    out
}

/// Strip a leading type-parameter DECLARATION modifier (`const`, `in`,
/// `out`) from a parameter name segment. These are legal only in the
/// declaration list (`<const T>`, `<in T>`, `<out U>`); at an
/// instantiation site (`typeof foo<T, U>`) only the bare name is valid.
/// Upstream uses `param.name.getText()`, which never includes them.
fn strip_tp_modifiers(name: &str) -> &str {
    let mut s = name.trim();
    // At most one of these realistically applies, but loop harmlessly.
    loop {
        let rest = s
            .strip_prefix("const ")
            .or_else(|| s.strip_prefix("in "))
            .or_else(|| s.strip_prefix("out "));
        match rest {
            Some(r) => s = r.trim_start(),
            None => return s,
        }
    }
}

/// Extract the `generics=` attribute value from the instance `<script>`
/// if present.
///
/// Svelte 5's syntax for declaring generic type params on a component:
///
/// ```svelte
/// <script lang="ts" generics="T extends Item, K extends keyof T">
/// ```
///
/// The value is spliced verbatim into our wrapping function as
/// `function $$render<T extends Item, K extends keyof T>() { ... }` so
/// any references to `T` / `K` inside the script body resolve correctly.
/// Blank out `type NAME = $$Generic[<args>];` declarations from a
/// script body, replacing each matched span with whitespace of equal
/// length so subsequent line/column source maps stay aligned.
///
/// Used in the rewrite chain when `synthesise_generics_from_dollar_generic`
/// has lifted these declarations into the render-fn's generic-param
/// list — the body must NOT also re-declare them (TS2300 duplicate
/// identifier in the function scope, on top of the local declaration
/// shadowing the generic parameter and degrading binding precision).
pub(crate) fn blank_dollar_generic_decls(script: &str) -> String {
    let bytes = script.as_bytes();
    let mut out = script.to_string();
    let mut cursor = 0;
    while cursor < bytes.len() {
        let Some(rel) = bytes[cursor..].windows(4).position(|w| w == b"type") else {
            break;
        };
        let kw_start = cursor + rel;
        cursor = kw_start + 4;
        // Reject identifier prefix/suffix continuation.
        let before_ok = kw_start == 0 || !is_ident_byte(bytes[kw_start - 1]);
        let after_ok = cursor < bytes.len() && is_ascii_ws(bytes[cursor]);
        if !before_ok || !after_ok {
            continue;
        }
        // Skip whitespace, read NAME.
        while cursor < bytes.len() && is_ascii_ws(bytes[cursor]) {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len() && is_ident_byte(bytes[cursor]) {
            cursor += 1;
        }
        if name_start == cursor {
            continue;
        }
        while cursor < bytes.len() && is_ascii_ws(bytes[cursor]) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && is_ascii_ws(bytes[cursor]) {
            cursor += 1;
        }
        if !script[cursor..].starts_with("$$Generic") {
            continue;
        }
        cursor += "$$Generic".len();
        // Optional `<args>`.
        if bytes.get(cursor) == Some(&b'<') {
            cursor += 1;
            let mut depth = 1usize;
            while cursor < bytes.len() && depth > 0 {
                match bytes[cursor] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
            if depth != 0 {
                return out;
            }
            cursor += 1; // past `>`
        }
        // Skip whitespace, expect `;` (or end of line / file).
        while cursor < bytes.len() && matches!(bytes[cursor], b' ' | b'\t') {
            cursor += 1;
        }
        let semi_end = if bytes.get(cursor) == Some(&b';') {
            cursor + 1
        } else {
            cursor
        };
        // Replace the span [kw_start..semi_end) with spaces, preserving
        // newlines so line numbers stay aligned.
        let span = &script[kw_start..semi_end];
        let replacement: String = span
            .chars()
            .map(|c| if c == '\n' || c == '\r' { c } else { ' ' })
            .collect();
        out.replace_range(kw_start..semi_end, &replacement);
        cursor = semi_end;
    }
    out
}

pub(crate) fn extract_generics_attr(doc: &Document<'_>) -> Option<SmolStr> {
    let script = doc.instance_script.as_ref()?;
    if let Some(g) = script.generics.as_deref() {
        return Some(SmolStr::from(g));
    }
    // SVELTE-4-COMPAT: when no `<script generics="...">` attribute is
    // present, fall back to scanning for `type NAME = $$Generic[<args>];`
    // declarations and synthesise a generic-parameter list. Mirrors
    // upstream svelte2tsx's `Generics.ts` which threads `$$Generic`
    // type names through to the render fn's `<...>` so consumer-side
    // `<Comp prop={value}>` calls bind the generic from the prop's
    // type. Without this, `type A = $$Generic` resolves at module
    // scope as `$$Generic<any> = any`, defeating per-call binding.
    synthesise_generics_from_dollar_generic(script.content)
}

/// Scan an instance-script body for `type NAME = $$Generic[<args>];`
/// declarations and return them as a generic-parameter list. Each
/// declaration becomes one parameter:
///   `type A = $$Generic;`            → `A`
///   `type B = $$Generic<keyof A>;`   → `B extends keyof A`
///   `type C = $$Generic<boolean>;`   → `C extends boolean`
///
/// Walk order is source order, so parameters reference each other as
/// in the user's source (`B extends keyof A` requires A first).
/// Returns `None` when no `$$Generic` declarations exist (caller's
/// non-`<script generics>` path keeps its existing behaviour).
fn synthesise_generics_from_dollar_generic(script: &str) -> Option<SmolStr> {
    let bytes = script.as_bytes();
    let mut params: Vec<String> = Vec::new();
    let mut cursor = 0;
    while cursor < bytes.len() {
        // Find next `type` keyword.
        let Some(rel) = bytes[cursor..].windows(4).position(|w| w == b"type") else {
            break;
        };
        let kw_start = cursor + rel;
        cursor = kw_start + 4;
        // Reject identifier prefix/suffix continuation
        // (`Type` / `prototype`).
        let before_ok = kw_start == 0 || !is_ident_byte(bytes[kw_start - 1]);
        let after_ok = cursor < bytes.len() && is_ascii_ws(bytes[cursor]);
        if !before_ok || !after_ok {
            continue;
        }
        // Skip whitespace, read NAME.
        while cursor < bytes.len() && is_ascii_ws(bytes[cursor]) {
            cursor += 1;
        }
        let name_start = cursor;
        while cursor < bytes.len() && is_ident_byte(bytes[cursor]) {
            cursor += 1;
        }
        if name_start == cursor {
            continue;
        }
        let name = &script[name_start..cursor];
        // Skip whitespace, expect `=`.
        while cursor < bytes.len() && is_ascii_ws(bytes[cursor]) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            continue;
        }
        cursor += 1;
        while cursor < bytes.len() && is_ascii_ws(bytes[cursor]) {
            cursor += 1;
        }
        // Expect literal `$$Generic` (followed by optional `<args>`).
        if !script[cursor..].starts_with("$$Generic") {
            continue;
        }
        cursor += "$$Generic".len();
        // Optional `<args>` — track angle-bracket nesting.
        let constraint = if bytes.get(cursor) == Some(&b'<') {
            cursor += 1;
            let arg_start = cursor;
            let mut depth = 1usize;
            while cursor < bytes.len() && depth > 0 {
                match bytes[cursor] {
                    b'<' => depth += 1,
                    b'>' => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
                cursor += 1;
            }
            if depth != 0 {
                return None;
            }
            let constraint = script[arg_start..cursor].trim().to_string();
            cursor += 1; // past `>`
            // A top-level comma means the user wrote `$$Generic<X, Y>`,
            // which has no single-constraint meaning. Splicing it would
            // emit malformed `A extends X, Y`. Upstream rejects this with
            // a transform error; lacking that error path here, abort the
            // whole synthesis so no malformed generic list is produced.
            if has_top_level_comma(&constraint) {
                return None;
            }
            if constraint.is_empty() {
                None
            } else {
                Some(constraint)
            }
        } else {
            None
        };
        match constraint {
            Some(c) => params.push(format!("{name} extends {c}")),
            None => params.push(name.to_string()),
        }
    }
    if params.is_empty() {
        None
    } else {
        Some(SmolStr::from(params.join(", ")))
    }
}

/// True if `s` contains a comma outside any `<...>` nesting. Used to
/// detect a multi-argument `$$Generic<X, Y>` whose span can't be spliced
/// as a single constraint. Tracks angle-bracket depth so a nested
/// `Map<X, Y>` does not false-positive.
fn has_top_level_comma(s: &str) -> bool {
    let mut depth: i32 = 0;
    for b in s.bytes() {
        match b {
            b'<' => depth += 1,
            b'>' => depth -= 1,
            b',' if depth == 0 => return true,
            _ => {}
        }
    }
    false
}

#[inline]
pub(crate) fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

/// Horizontal whitespace only: space and tab. Used by declarator
/// scanners that need to know when a `\n` ends a statement vs.
/// continues it — `is_ascii_ws` is too greedy for that, since a
/// blanket newline skip after the name lets `let X\nlet Y = init`
/// be misread as a single `let X = init` (with the next line's `=`
/// attributed to the previous line's `X`). Pair with a deliberate
/// `\n` continuation check at each call site so multi-line `let X:
///     Type = init` shapes still parse as one declarator.
#[inline]
pub(crate) fn is_horiz_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t')
}

#[inline]
pub(crate) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

/// True for CSS-custom-property attribute names (`--foo`, `--some-var`).
/// Svelte 5 treats `<Comp --css-var={...}>` as a CSS variable on the
/// component's wrapper element, not as a typed prop — so the emit
/// routes these through `__svn_css_prop` which returns `{}` and
/// doesn't contribute to the Props object type.
pub(crate) fn is_css_custom_prop_name(name: &str) -> bool {
    name.starts_with("--")
}

/// True for ASCII identifiers `[A-Za-z_$][A-Za-z0-9_$]*`. We don't try
/// to enumerate JS reserved words — modern JS (ES5+) allows reserved
/// words as bare property names anyway.
pub(crate) fn is_simple_js_identifier(s: &str) -> bool {
    let mut chars = s.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    if !(first.is_ascii_alphabetic() || first == '_' || first == '$') {
        return false;
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

#[cfg(test)]
mod generic_arg_names_tests {
    use super::generic_arg_names;

    #[test]
    fn strips_constraints_and_defaults() {
        assert_eq!(generic_arg_names("T extends string, U = number"), "T, U");
    }

    #[test]
    fn strips_declaration_modifiers() {
        // `const` / `in` / `out` are declaration-only; instantiation
        // sites take the bare name (upstream `param.name.getText()`).
        assert_eq!(generic_arg_names("const T extends readonly string[]"), "T");
        assert_eq!(generic_arg_names("in T, out U"), "T, U");
        assert_eq!(generic_arg_names("const T"), "T");
    }

    #[test]
    fn plain_names_unchanged() {
        assert_eq!(generic_arg_names("T, U, V"), "T, U, V");
    }

    #[test]
    fn preserves_non_ascii_name_bytes() {
        assert_eq!(generic_arg_names("Ψ extends number"), "Ψ");
    }
}
