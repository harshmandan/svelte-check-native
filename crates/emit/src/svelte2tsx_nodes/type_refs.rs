//! `keyof typeof <ident>` / `typeof <ident>` byte-scan helpers.
//!
//! Mirrors the hoistability-decision half of upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/HoistableInterfaces.ts`
//! (the part that checks whether a type alias / interface references
//! a body-local identifier via `typeof`).
//!
//! Called by [`crate::process_instance_script_content`] when deciding
//! whether a `type`/`interface` declaration can be hoisted to module
//! scope. Two policies:
//!
//! - `keyof typeof <name>` — the declare-const stub
//!   (`{ [k: string]: any }`) loses literal-key precision. Such types
//!   must stay body-scoped to preserve `keyof` against the real const.
//! - bare `typeof <name>` — the plain-`any` stub loses all structural
//!   info, cascading into implicit-any errors. Same body-scoped
//!   policy.

use smol_str::SmolStr;

/// Byte-scan a JS/TS source slice for `keyof typeof IDENT` targets
/// — the names that appear in the specific `keyof typeof <name>`
/// shape where the intervening whitespace allows arbitrary spacing.
/// This pattern is special because it's the one form our declare-
/// const stub can't preserve: stubbed `{ [k: string]: any }` has
/// `keyof = string | number`, losing the real const's literal-key
/// precision.
pub(crate) fn keyof_typeof_targets(text: &str) -> Vec<SmolStr> {
    typeof_targets_inner(text, true)
}

/// Collect every identifier following a bare `typeof` keyword in a
/// type position (no preceding `keyof`, `&`, `|`, etc. are fine — we
/// just need the symbol name). Used with the plain-`any` hoisted-stub
/// policy: when a type alias references `typeof <body-local>`, the
/// module-scope stub loses all structural info (it's `any`), so the
/// user's `type X = typeof moments` resolves to `any` and downstream
/// cascades into implicit-any errors. Keeping such types body-scoped
/// preserves the real `typeof` reference against the live `let`.
pub(crate) fn typeof_targets(text: &str) -> Vec<SmolStr> {
    typeof_targets_inner(text, false)
}

fn typeof_targets_inner(text: &str, require_keyof: bool) -> Vec<SmolStr> {
    let bytes = text.as_bytes();
    let mut out: Vec<SmolStr> = Vec::new();
    let mut i = 0usize;
    while i < bytes.len() {
        let start_of_typeof_clause = if require_keyof {
            if !bytes[i..].starts_with(b"keyof") {
                i += 1;
                continue;
            }
            let before_ok = i == 0 || !is_ident_byte(bytes[i - 1]);
            let after_keyof = i + b"keyof".len();
            let after_ok = after_keyof < bytes.len() && !is_ident_byte(bytes[after_keyof]);
            if !(before_ok && after_ok) {
                i += 1;
                continue;
            }
            let mut j = after_keyof;
            while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                j += 1;
            }
            j
        } else {
            i
        };
        if bytes[start_of_typeof_clause..].starts_with(b"typeof") {
            // `typeof` must not be preceded by an ident char AND must
            // be followed by a non-ident char.
            let before_ok =
                start_of_typeof_clause == 0 || !is_ident_byte(bytes[start_of_typeof_clause - 1]);
            let after_typeof = start_of_typeof_clause + b"typeof".len();
            let after_ok = after_typeof < bytes.len() && !is_ident_byte(bytes[after_typeof]);
            if before_ok && after_ok {
                let mut k = after_typeof;
                while k < bytes.len() && bytes[k].is_ascii_whitespace() {
                    k += 1;
                }
                if k < bytes.len()
                    && (bytes[k].is_ascii_alphabetic() || bytes[k] == b'_' || bytes[k] == b'$')
                {
                    let start = k;
                    while k < bytes.len() && is_ident_byte(bytes[k]) {
                        k += 1;
                    }
                    out.push(SmolStr::from(&text[start..k]));
                    i = k;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

#[inline]
fn is_ident_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'$'
}
