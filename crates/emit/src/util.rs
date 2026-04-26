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
pub(crate) fn generic_arg_names(generics: &str) -> String {
    let mut out = String::new();
    let mut depth_angle: i32 = 0;
    let mut depth_paren: i32 = 0;
    let mut depth_bracket: i32 = 0;
    let mut current_name = String::new();
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
            let trimmed = current_name.trim();
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
                current_name.push(b as char);
                i += 1;
                continue;
            }
            if b == b'=' {
                in_name = false;
                i += 1;
                continue;
            }
            current_name.push(b as char);
        }
        i += 1;
    }
    let trimmed = current_name.trim();
    if !trimmed.is_empty() {
        if !out.is_empty() {
            out.push_str(", ");
        }
        out.push_str(trimmed);
    }
    out
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
pub(crate) fn extract_generics_attr(doc: &Document<'_>) -> Option<SmolStr> {
    let script = doc.instance_script.as_ref()?;
    script.generics.as_deref().map(SmolStr::from)
}

#[inline]
pub(crate) fn is_ascii_ws(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r')
}

#[inline]
pub(crate) fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}

#[inline]
pub(crate) fn utf8_char_len(b: u8) -> usize {
    if b < 0xC0 {
        1
    } else if b < 0xE0 {
        2
    } else if b < 0xF0 {
        3
    } else {
        4
    }
}
