//! Runes-mode detection at the source-text level.
//!
//! Svelte switches a component into "runes mode" the moment a rune
//! (`$state`, `$derived`, `$effect`, `$props`, `$bindable`, `$inspect`,
//! `$host`) is *called*. Detecting this is a "does a `$rune(` appear
//! anywhere in the script" question — for which a comment/string/
//! template-aware text scan is strictly more complete than an AST
//! descent (an incomplete descent would MISS a rune call nested in some
//! expression position and silently pick the wrong mode). That makes
//! this one of the sanctioned text-scan heuristics rather than an
//! `oxc` AST walk (architecture rule #1). The state machine skips line
//! comments, block comments, string literals, and template-literal text
//! — recursing into each `${…}` interpolation — so a `$state(` written
//! inside a comment or string never falsely flips the mode.

const MARKERS: &[&[u8]] = &[
    b"$state",
    b"$derived",
    b"$effect",
    b"$props",
    b"$bindable",
    b"$inspect",
    b"$host",
];

/// Returns true if `source` contains a rune CALL (`$rune(` or
/// `$rune.method(`) outside any comment, string, or template-literal
/// text. The `(` requirement excludes the `$$props`/`$$restProps`
/// ambients and a non-rune `import { state as $state }` alias used as a
/// bare identifier.
pub fn script_calls_rune(source: &str) -> bool {
    scan(source.as_bytes())
}

fn scan(bytes: &[u8]) -> bool {
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        // Line comment.
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // String literal — escape-aware.
        if b == b'\'' || b == b'"' {
            let quote = b;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else {
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            continue;
        }
        // Template literal — skip the literal text but recurse on each
        // `${…}` interpolation so a rune call inside one still counts.
        if b == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
                    let interp_start = i + 2;
                    let end = find_interpolation_end(bytes, interp_start).unwrap_or(bytes.len());
                    if scan(&bytes[interp_start..end]) {
                        return true;
                    }
                    i = end.saturating_add(1).min(bytes.len());
                    continue;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            continue;
        }
        // Try matching a rune marker at this code position.
        for marker in MARKERS {
            if bytes[i..].starts_with(marker) {
                // `$$props` ambient: previous char must not be `$`.
                let prev = i.checked_sub(1).and_then(|p| bytes.get(p)).copied();
                if prev != Some(b'$') {
                    let mut after = i + marker.len();
                    // Consume `.word` chains (`$state.raw`, `$derived.by`).
                    while bytes.get(after) == Some(&b'.') {
                        after += 1;
                        while after < bytes.len()
                            && (bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_')
                        {
                            after += 1;
                        }
                    }
                    while after < bytes.len() && matches!(bytes[after], b' ' | b'\t') {
                        after += 1;
                    }
                    if bytes.get(after) == Some(&b'(') {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

/// Locate the `}` that closes a `${` interpolation that started at
/// `start` (one level of `{` already open). Skips comments, strings, and
/// nested template literals so a `}` inside one can't end it early.
fn find_interpolation_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut j = start;
    while j < bytes.len() {
        let c = bytes[j];
        if c == b'/' && bytes.get(j + 1) == Some(&b'/') {
            while j < bytes.len() && bytes[j] != b'\n' {
                j += 1;
            }
            continue;
        }
        if c == b'/' && bytes.get(j + 1) == Some(&b'*') {
            j += 2;
            while j + 1 < bytes.len() && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                j += 1;
            }
            j = (j + 2).min(bytes.len());
            continue;
        }
        if c == b'\'' || c == b'"' {
            j += 1;
            while j < bytes.len() && bytes[j] != c {
                if bytes[j] == b'\\' {
                    j = (j + 2).min(bytes.len());
                } else {
                    j += 1;
                }
            }
            j = (j + 1).min(bytes.len());
            continue;
        }
        if c == b'`' {
            j += 1;
            while j < bytes.len() && bytes[j] != b'`' {
                if bytes[j] == b'\\' {
                    j = (j + 2).min(bytes.len());
                    continue;
                }
                if bytes[j] == b'$' && bytes.get(j + 1) == Some(&b'{') {
                    let inner_start = j + 2;
                    j = match find_interpolation_end(bytes, inner_start) {
                        Some(pos) => (pos + 1).min(bytes.len()),
                        None => bytes.len(),
                    };
                    continue;
                }
                j += 1;
            }
            j = (j + 1).min(bytes.len());
            continue;
        }
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::script_calls_rune;

    #[test]
    fn detects_plain_rune_call() {
        assert!(script_calls_rune("let x = $state(0);"));
        assert!(script_calls_rune("let x = $derived.by(() => 1);"));
    }

    #[test]
    fn ignores_rune_in_line_comment() {
        assert!(!script_calls_rune(
            "// migrate to $state(0) later\nlet x = 1;"
        ));
    }

    #[test]
    fn ignores_rune_in_block_comment() {
        assert!(!script_calls_rune("/* $state(0) */ let x = 1;"));
    }

    #[test]
    fn ignores_rune_in_string() {
        assert!(!script_calls_rune("const s = \"$state(0)\";"));
        assert!(!script_calls_rune("const s = 'use $props() here';"));
    }

    #[test]
    fn detects_rune_in_template_interpolation() {
        assert!(script_calls_rune("const t = `${$state(0)}`;"));
    }

    #[test]
    fn ignores_rune_in_template_text() {
        assert!(!script_calls_rune("const t = `cost is $state(x)`;"));
    }

    #[test]
    fn rejects_dollar_dollar_ambient_and_bare_identifier() {
        assert!(!script_calls_rune("const p = $$props;"));
        assert!(!script_calls_rune(
            "import { state as $state } from 'x'; let y = $state;"
        ));
    }

    #[test]
    fn rejects_identifier_suffix_continuation() {
        assert!(!script_calls_rune("function $stateful() {} $stateful();"));
    }
}
