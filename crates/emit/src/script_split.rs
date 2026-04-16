//! Hoist `import` declarations out of an instance script body.
//!
//! `<script>` content in a Svelte 5 component is module-scope code, but
//! our emit wraps it in `function $$render() { ... }`. ES `import`s are
//! illegal inside a function body (TS1232), so we lift them to module
//! top level and blank the original spans with whitespace. Code in the
//! body still sees the imports via normal module scope.
//!
//! Replacing instead of deleting preserves byte offsets inside the body
//! so line/column positions stay aligned for the source-map mapping
//! that runs later.
//!
//! Note: this module no longer strips `export` modifiers (Svelte 4 prop
//! syntax `export let foo`). Svelte 5 uses `let { foo } = $props()` for
//! props — there's nothing to strip.

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use svn_parser::{ScriptLang, parse_script_body};

/// `imports`: text to insert at module top-level (one declaration per
/// segment, joined with newlines).
/// `body`: the original script content with import regions blanked out.
#[derive(Debug, Clone)]
pub struct SplitScript {
    pub imports: String,
    pub body: String,
}

/// Split out the top-level `import` declarations from a script body.
///
/// Re-parses the body with oxc; cheap enough at the small sizes a
/// `<script>` block holds. If parsing fails (malformed user code) the
/// content is passed through unchanged in `body`, with `imports` empty.
pub fn split_imports(content: &str, lang: ScriptLang) -> SplitScript {
    if !content.contains("import") {
        return SplitScript {
            imports: String::new(),
            body: content.to_string(),
        };
    }

    let allocator = Allocator::default();
    let parsed = parse_script_body(&allocator, content, lang);

    if parsed.panicked {
        return SplitScript {
            imports: String::new(),
            body: content.to_string(),
        };
    }

    let mut import_spans: Vec<(usize, usize)> = Vec::new();
    for stmt in &parsed.program.body {
        if let Statement::ImportDeclaration(decl) = stmt {
            import_spans.push((decl.span.start as usize, decl.span.end as usize));
        }
    }

    if import_spans.is_empty() {
        return SplitScript {
            imports: String::new(),
            body: content.to_string(),
        };
    }

    let mut imports = String::new();
    for &(start, end) in &import_spans {
        imports.push_str(&content[start..end]);
        if !content[start..end].ends_with('\n') {
            imports.push('\n');
        }
    }

    // Replacing each import span with ASCII whitespace of the same byte
    // length preserves every byte offset after it — keeps source-map
    // positions accurate.
    let mut body = String::with_capacity(content.len());
    let mut cursor = 0;
    for &(start, end) in &import_spans {
        body.push_str(&content[cursor..start]);
        for ch in content[start..end].chars() {
            if ch == '\n' || ch == '\r' {
                body.push(ch);
            } else if ch.is_ascii() {
                body.push(' ');
            } else {
                let byte_len = ch.len_utf8();
                for _ in 0..byte_len {
                    body.push(' ');
                }
            }
        }
        cursor = end;
    }
    body.push_str(&content[cursor..]);

    SplitScript { imports, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_imports_passes_through() {
        let s = split_imports("let x = 1;", ScriptLang::Js);
        assert_eq!(s.imports, "");
        assert_eq!(s.body, "let x = 1;");
    }

    #[test]
    fn single_import_is_hoisted() {
        let src = "import { writable } from 'svelte/store';\nlet x = 1;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(
            s.imports
                .contains("import { writable } from 'svelte/store';")
        );
        assert!(s.body.contains("let x = 1;"));
        // Hoisted region replaced with whitespace, not removed — preserves
        // line offsets for the body content that follows.
        assert!(!s.body.contains("import"));
    }

    #[test]
    fn multiple_imports_all_hoisted() {
        let src = "\
import a from 'a';
import b from 'b';
let x = 1;
";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.imports.contains("import a from 'a';"));
        assert!(s.imports.contains("import b from 'b';"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn type_only_imports_hoisted() {
        let src = "import type { Foo } from './foo';\nlet x: Foo = bar;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.imports.contains("import type { Foo }"));
    }

    #[test]
    fn body_offsets_preserved_for_source_mapping() {
        // After hoisting, the byte position of `let x` in body should match
        // its position in the original content (not shifted earlier).
        let src = "import a from 'a';\nlet x = 1;";
        let original_pos = src.find("let x").unwrap();
        let s = split_imports(src, ScriptLang::Ts);
        let new_pos = s.body.find("let x").unwrap();
        assert_eq!(
            new_pos, original_pos,
            "blanking should preserve byte offsets so line/col mapping stays valid"
        );
    }

    #[test]
    fn newlines_inside_import_preserved() {
        // Multi-line import shouldn't collapse to one line — keep newlines
        // so subsequent line numbers don't shift.
        let src = "\
import {
    a,
    b,
} from 'mod';
let x = 1;
";
        let original_let_line = src.lines().position(|l| l.contains("let x")).unwrap();
        let s = split_imports(src, ScriptLang::Ts);
        let new_let_line = s.body.lines().position(|l| l.contains("let x")).unwrap();
        assert_eq!(new_let_line, original_let_line);
    }

    #[test]
    fn malformed_script_falls_back_to_passthrough() {
        // If oxc panics, return the source unchanged.
        let src = "import {{{ unbalanced";
        let s = split_imports(src, ScriptLang::Ts);
        // `body` should contain the original; `imports` may or may not
        // have content depending on whether oxc recovered enough to find
        // a valid import statement.
        let total = format!("{}{}", s.imports, s.body);
        assert!(total.contains("import"));
    }

    #[test]
    fn no_imports_fast_path() {
        let s = split_imports("const x = 1;", ScriptLang::Ts);
        assert_eq!(s.imports, "");
        assert_eq!(s.body, "const x = 1;");
    }
}
