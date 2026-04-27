//! Pull every binding-introducing identifier out of a destructure
//! pattern's source text.
//!
//! Used by emit when it needs the names a `{#each items as <pat>}`,
//! `{#snippet name(<params>)}`, `{:then <pat>}`, or `{:catch <pat>}`
//! introduces — emit declares each as `let <name>: any;` inside the
//! enclosing scope so descendant template references resolve.
//!
//! ## Why this is byte-scanning, not AST
//!
//! Architecture rule #1 (CLAUDE.md) says embedded JS/TS goes
//! through `oxc_parser`. This helper is an exception flagged in
//! its own doc comment: the patterns that show up at template-
//! binding sites are a tiny grammar subset (identifiers, object/
//! array destructure, optional defaults, optional TS type
//! annotations on snippet params), and oxc invocation per each-
//! block on a 1000+-component file is the kind of overhead the
//! `walk_template` hot path is sensitive to. The byte scanner
//! tracks just enough state — brace depth + a few special
//! characters — to handle the shapes Svelte's parser will accept.
//!
//! If a future shape turns out to need AST fidelity (a hand-
//! crafted bug repro that this scanner mishandles), the right
//! response is moving the call to oxc here, not adding more
//! special cases to the scanner.

/// Pull every identifier-like token out of a destructuring pattern.
///
/// For `id`              → `["id"]`
/// For `[id, label]`     → `["id", "label"]`
/// For `[id, { label }]` → `["id", "label"]`
/// For `{ a: x, b }`     → `["x", "b"]` (only the local-name side of `key:value`)
///
/// Falls back to a single `__svn_each_unused` token when nothing
/// identifier-shaped is found, so the emitted `void` line stays
/// valid.
pub(crate) fn all_identifiers(binding: &str) -> Vec<String> {
    let bytes = binding.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut depth_brace = 0usize; // tracks `{ ... }` for object key:value
    while i < bytes.len() {
        let b = bytes[i];
        match b {
            b'{' => {
                depth_brace += 1;
                i += 1;
            }
            b'}' => {
                depth_brace = depth_brace.saturating_sub(1);
                i += 1;
            }
            b'?' if depth_brace == 0 => {
                // Optional-parameter marker in TS snippet params: `name?:
                // Type`. No-op; the following `:` handles the type skip.
                // Each-block `as` clauses don't use `?`, so this branch
                // is inert for that caller.
                i += 1;
            }
            b':' if depth_brace == 0 => {
                // Top-level `:` on a snippet parameter introduces a TS
                // type annotation (`name: Foo<Bar>`). Skip until the
                // next top-level `,` — tracking paren/bracket/brace/
                // angle nesting so commas inside `Array<A, B>` or
                // `(a: X) => Y` don't terminate the annotation early.
                // Each-block `as` clauses never hit this (Svelte grammar
                // forbids type annotations on destructure targets).
                i += 1;
                let mut depth = 0usize;
                while i < bytes.len() {
                    match bytes[i] {
                        b'(' | b'[' | b'{' | b'<' => depth += 1,
                        b')' | b']' | b'}' | b'>' if depth > 0 => depth -= 1,
                        b',' if depth == 0 => break,
                        _ => {}
                    }
                    i += 1;
                }
            }
            b'=' => {
                // Skip default value `name = expr` — advance past expr to
                // the next comma/closer at the same depth. Conservative:
                // just stop collecting until we see `,` `]` `}`.
                i += 1;
                let mut paren = 0usize;
                while i < bytes.len() {
                    match bytes[i] {
                        b'(' | b'[' | b'{' => paren += 1,
                        b')' | b']' | b'}' if paren > 0 => paren -= 1,
                        b',' | b']' | b'}' if paren == 0 => break,
                        _ => {}
                    }
                    i += 1;
                }
            }
            _ if b.is_ascii_alphabetic() || b == b'_' || b == b'$' => {
                let start = i;
                while i < bytes.len() {
                    let c = bytes[i];
                    if c.is_ascii_alphanumeric() || c == b'_' || c == b'$' {
                        i += 1;
                    } else {
                        break;
                    }
                }
                let name = &binding[start..i];
                let take = if depth_brace > 0 {
                    // Inside an object pattern: `key: local` — only collect
                    // the local. `{ a }` shorthand has no colon so `a`
                    // counts. Look ahead: if the next non-ws byte is `:`,
                    // this identifier is a key and must be skipped;
                    // otherwise it is a binding (either the local after
                    // a colon, or a shorthand entry).
                    let mut j = i;
                    while j < bytes.len() && bytes[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    bytes.get(j) != Some(&b':')
                } else {
                    true
                };
                if take {
                    out.push(name.to_string());
                }
            }
            _ => {
                i += 1;
            }
        }
    }
    if out.is_empty() {
        out.push("__svn_each_unused".to_string());
    }
    out
}
