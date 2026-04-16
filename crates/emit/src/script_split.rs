//! Normalize an instance script body so it can be wrapped in
//! `function $$render() { ... }` without producing TypeScript errors.
//!
//! Two transforms, both done in a single oxc parse pass:
//!
//! 1. **Hoist `import` declarations.** ES `import`s are illegal inside a
//!    function body (TS1232). We lift them out to module top level and
//!    blank the original spans with whitespace. Code in the body still
//!    sees them via normal module scope.
//! 2. **Strip `export` modifiers** from top-level `export let foo` /
//!    `export const foo` declarations. Svelte 4 used `export let foo` to
//!    declare a prop; that keyword is illegal inside a function body
//!    (TS1184 "Modifiers cannot appear here"). For type-checking
//!    purposes the `export` is meaningless — the prop semantics come
//!    from svelte2tsx's wrapping, not from the keyword. We blank just
//!    the `export ` (six bytes) so the `let foo` that follows still
//!    parses.
//!
//! Both transforms preserve byte offsets inside the body (we replace,
//! not delete) so line/column positions stay aligned for the
//! source-map mapping that runs later.

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_span::GetSpan;
use svn_parser::{ScriptLang, parse_script_body};

/// `imports`: text to insert at module top-level (one declaration per
/// segment, joined with newlines).
/// `body`: the original script content with import regions blanked out.
#[derive(Debug, Clone)]
pub struct SplitScript {
    pub imports: String,
    pub body: String,
}

/// Split + normalize an instance script body.
///
/// - Top-level `import` declarations are lifted out into `imports` and
///   blanked from the body.
/// - Top-level `export` modifiers on `let`/`const`/`var` declarations
///   are blanked from the body so the resulting `let foo: T` can be
///   wrapped in a function without TS1184.
///
/// Re-parses the body once with oxc. If parsing panics (malformed user
/// code), the content is passed through unchanged.
pub fn split_imports(content: &str, lang: ScriptLang) -> SplitScript {
    let needs_pass = content.contains("import") || content.contains("export");
    if !needs_pass {
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
    let mut export_keyword_spans: Vec<(usize, usize)> = Vec::new();

    for stmt in &parsed.program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                import_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportNamedDeclaration(decl) if decl.declaration.is_some() => {
                // `export let foo` / `export const foo` / `export var foo`.
                // We only blank the `export ` keyword (and the trailing
                // whitespace up to the declaration body) so the
                // `let foo: T = bar;` part stays intact and parseable
                // inside the wrapping function.
                if let Some(d) = &decl.declaration {
                    let stmt_start = decl.span.start as usize;
                    let inner_start = GetSpan::span(d).start as usize;
                    if inner_start > stmt_start {
                        export_keyword_spans.push((stmt_start, inner_start));
                    }
                }
            }
            _ => {}
        }
    }

    if import_spans.is_empty() && export_keyword_spans.is_empty() {
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

    // Combine import + export-keyword spans into one sorted list, then
    // blank each region. Replacing with ASCII whitespace of the same
    // byte length preserves every byte offset after it — line/col stays
    // valid for the source-map mapping that runs later.
    let mut blank_spans: Vec<(usize, usize)> = Vec::new();
    blank_spans.extend(import_spans.iter().copied());
    blank_spans.extend(export_keyword_spans.iter().copied());
    blank_spans.sort_by_key(|&(s, _)| s);

    let mut body = String::with_capacity(content.len());
    let mut cursor = 0;
    for &(start, end) in &blank_spans {
        body.push_str(&content[cursor..start]);
        for ch in content[start..end].chars() {
            if ch == '\n' || ch == '\r' {
                body.push(ch);
            } else if ch.is_ascii() {
                body.push(' ');
            } else {
                // Multi-byte char: replace each byte with a space.
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
    fn no_import_or_export_keyword_fast_path() {
        let s = split_imports("const x = 1;", ScriptLang::Ts);
        assert_eq!(s.imports, "");
        assert_eq!(s.body, "const x = 1;");
    }

    #[test]
    fn export_let_modifier_stripped() {
        // Svelte 4 prop syntax: `export let foo` becomes plain `let foo`
        // for type-checking. The `export` keyword is illegal inside a
        // function body (TS1184) so it must come out.
        let src = "export let foo: string = 'hi';";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.imports.is_empty());
        assert!(
            !s.body.contains("export"),
            "body still contains `export`:\n{}",
            s.body
        );
        assert!(
            s.body.contains("let foo: string"),
            "the underlying declaration must survive intact:\n{}",
            s.body
        );
    }

    #[test]
    fn export_const_modifier_stripped() {
        let src = "export const PI = 3.14;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(!s.body.contains("export"));
        assert!(s.body.contains("const PI = 3.14;"));
    }

    #[test]
    fn export_with_destructuring_stripped() {
        let src = "export let { foo, bar }: { foo: string; bar: number } = obj;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(!s.body.contains("export"));
        assert!(s.body.contains("let { foo, bar }"));
    }

    #[test]
    fn re_export_statement_left_alone() {
        // `export { foo }` (no declaration) is a re-export; it doesn't have
        // a `declaration` field on ExportNamedDeclaration so our pass
        // leaves it alone. (It would still be invalid inside a function,
        // but stripping the keyword would change semantics meaningfully —
        // best to leave that case for a future iteration.)
        let src = "let x = 1;\nexport { x };";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(
            s.body.contains("export { x }"),
            "re-export without declaration should be left intact:\n{}",
            s.body
        );
    }

    #[test]
    fn export_offsets_preserved() {
        let src = "export let foo: T = 1;\nlet b = foo;";
        let original_b_pos = src.find("let b").unwrap();
        let s = split_imports(src, ScriptLang::Ts);
        let new_b_pos = s.body.find("let b").unwrap();
        assert_eq!(new_b_pos, original_b_pos);
    }

    #[test]
    fn import_and_export_in_same_script() {
        let src = "\
import { writable } from 'svelte/store';
export let count = writable(0);
";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.imports.contains("import { writable }"));
        assert!(!s.body.contains("export"));
        assert!(s.body.contains("let count = writable(0);"));
    }
}
