//! Identifier-reference byte-scanner for hoistability decisions.
//!
//! Mirrors the reference-graph half of upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/InterfacesAndTypes.ts`
//! and the related `HoistableInterfaces.ts` walker. Detects every
//! identifier that appears in a type-source slice so the caller can
//! intersect that set against the body-declared names — types whose
//! references all resolve to module-scope are safe to hoist.
//!
//! Called by [`crate::process_instance_script_content`] at multiple
//! sites (hoisting eligibility, type-alias scan, default-export
//! reference safety).

use std::collections::HashSet;

use smol_str::SmolStr;

/// Byte-scan a JS/TS source slice for identifier references.
///
/// Returns every identifier that appears NOT after a `.` or `?.` (so
/// `obj.prop` yields `obj`, not `prop`). Skips string literals,
/// template-literal text (but recurses into `${...}` substitutions),
/// and line/block comments. A keyword/built-in list is filtered out
/// so `typeof`, `keyof`, etc. don't leak into the result.
///
/// The scanner is intentionally lenient — false positives (e.g. a
/// property key in an object literal) are acceptable because the
/// caller intersects with a known set of body-declared names.
pub(crate) fn collect_ident_refs(text: &str) -> Vec<SmolStr> {
    let mut seen: HashSet<SmolStr> = HashSet::new();
    let mut out: Vec<SmolStr> = Vec::new();
    let bytes = text.as_bytes();
    let mut i = 0usize;
    let mut after_dot = false;

    while i < bytes.len() {
        let b = bytes[i];

        // Line comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            i += 2;
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            if i + 1 < bytes.len() {
                i += 2;
            }
            continue;
        }
        // String literal.
        if b == b'"' || b == b'\'' {
            let q = b;
            i += 1;
            while i < bytes.len() && bytes[i] != q {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            after_dot = false;
            continue;
        }
        // Template literal.
        if b == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                if bytes[i] == b'$' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
                    i += 2;
                    let inner_start = i;
                    let mut depth = 1usize;
                    while i < bytes.len() {
                        match bytes[i] {
                            b'{' => depth += 1,
                            b'}' => {
                                depth -= 1;
                                if depth == 0 {
                                    break;
                                }
                            }
                            _ => {}
                        }
                        i += 1;
                    }
                    let inner = &text[inner_start..i];
                    for sub in collect_ident_refs(inner) {
                        if seen.insert(sub.clone()) {
                            out.push(sub);
                        }
                    }
                    if i < bytes.len() {
                        i += 1; // past `}`
                    }
                } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    i += 2;
                } else {
                    i += 1;
                }
            }
            if i < bytes.len() {
                i += 1;
            }
            after_dot = false;
            continue;
        }
        // Identifier-like start.
        if b.is_ascii_alphabetic() || b == b'_' || b == b'$' || b >= 0x80 {
            let start = i;
            while i < bytes.len() {
                let c = bytes[i];
                if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' || c >= 0x80 {
                    i += 1;
                } else {
                    break;
                }
            }
            let name = &text[start..i];
            if !after_dot && !is_ref_scan_keyword(name) {
                let s = SmolStr::from(name);
                if seen.insert(s.clone()) {
                    out.push(s);
                }
            }
            after_dot = false;
            continue;
        }
        // Member access — suppress next identifier.
        if b == b'.' {
            after_dot = true;
            i += 1;
            continue;
        }
        if !b.is_ascii_whitespace() {
            after_dot = false;
        }
        i += 1;
    }

    out
}

/// Keywords/built-ins that appear frequently in TS type annotations
/// and should never be treated as a reference.
fn is_ref_scan_keyword(s: &str) -> bool {
    matches!(
        s,
        "true"
            | "false"
            | "null"
            | "undefined"
            | "this"
            | "void"
            | "typeof"
            | "keyof"
            | "infer"
            | "extends"
            | "in"
            | "of"
            | "as"
            | "is"
            | "let"
            | "const"
            | "var"
            | "function"
            | "if"
            | "else"
            | "for"
            | "while"
            | "do"
            | "return"
            | "yield"
            | "await"
            | "async"
            | "delete"
            | "throw"
            | "try"
            | "catch"
            | "finally"
            | "switch"
            | "case"
            | "default"
            | "break"
            | "continue"
            | "class"
            | "super"
            | "import"
            | "export"
            | "from"
            | "satisfies"
            | "readonly"
            | "type"
            | "interface"
            | "namespace"
            | "module"
            | "declare"
            | "public"
            | "private"
            | "protected"
            | "new"
            | "instanceof"
            | "any"
            | "unknown"
            | "never"
            | "number"
            | "string"
            | "boolean"
            | "symbol"
            | "object"
            | "bigint"
    )
}
