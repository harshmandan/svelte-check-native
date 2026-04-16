//! Hoist module-level statements out of an instance script body.
//!
//! `<script>` content in a Svelte 5 component is module-scope code, but our
//! emit wraps it in `function $$render() { ... }`. Several statement kinds
//! are illegal inside a function body and must be lifted to module top
//! level:
//!
//! - **`import`** — TS1232 if inside a function
//! - **`export const/let/var/function/class`** — TS1184 / TS1233
//! - **`export { a, b }` / `export { a as b }`** — TS1233
//! - **`export { a } from 'mod'`** — TS1233
//! - **`export default x`** — TS1232
//! - **`export * from 'mod'`** — TS1232
//!
//! All are hoisted to a module-level prelude. The original spans inside
//! the script body are blanked with whitespace of the same byte length so
//! line/column positions inside the body stay aligned for source-map
//! mapping.

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use oxc_span::GetSpan;
use svn_parser::{ScriptLang, parse_script_body};

/// `hoisted`: statements lifted to module top level (newline-joined).
/// `body`: the original script content with hoisted spans blanked out.
#[derive(Debug, Clone)]
pub struct SplitScript {
    pub hoisted: String,
    pub body: String,
}

/// Split out every module-level statement (imports, exports of all
/// shapes) from a script body.
///
/// Re-parses the body once with oxc. If parsing panics on malformed user
/// code, the content is passed through unchanged.
pub fn split_imports(content: &str, _lang: ScriptLang) -> SplitScript {
    // Fast path: no import/export keyword at all → nothing to hoist.
    if !content.contains("import") && !content.contains("export") {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
        };
    }

    // Always parse as TypeScript — TS is a superset of JS for our
    // purposes (we're identifying statement spans, not generating
    // runtime code). Parsing as TS lets us correctly handle scripts
    // that use type annotations even when `<script>` doesn't carry
    // `lang="ts"`. (Svelte 5 + svelte:options runes accepts this.)
    let allocator = Allocator::default();
    let parsed = parse_script_body(&allocator, content, ScriptLang::Ts);

    if parsed.panicked {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
        };
    }

    // Spans we hoist verbatim to module top level. For statements that
    // are pure module-shape (no references to body locals): imports,
    // `export { x } from 'mod'`, `export * from 'mod'`.
    let mut hoist_spans: Vec<(usize, usize)> = Vec::new();
    // Spans where we strip just the `export ` prefix and let the inner
    // declaration stay in the body. For `export const/let/var/function/class`
    // — the declaration body might reference locals (e.g. `export function
    // getA() { return a; }` where `a` is a local), so hoisting would
    // break those references. Stripping the keyword keeps everything in
    // scope; the consumer-facing export goes away (consumers can't
    // `import { foo } from './X.svelte'` for these names) but the body
    // type-checks cleanly.
    let mut strip_keyword_spans: Vec<(usize, usize)> = Vec::new();
    // Spans we drop entirely (blank in body, don't add to hoisted prelude).
    // For `export { x, y }` (no `from`) re-exports of local names, and
    // `export default x` where x is a name (we can't easily distinguish
    // expression-vs-name without more parsing — drop is safer).
    let mut drop_spans: Vec<(usize, usize)> = Vec::new();

    for stmt in &parsed.program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportNamedDeclaration(decl) => {
                let span = (decl.span.start as usize, decl.span.end as usize);
                if let Some(d) = &decl.declaration {
                    // `export const/let/var/function/class` — strip just
                    // the `export ` prefix. The declaration content stays
                    // in body where its identifier references resolve.
                    let inner_start = GetSpan::span(d).start as usize;
                    if inner_start > span.0 {
                        strip_keyword_spans.push((span.0, inner_start));
                    }
                } else if decl.source.is_some() {
                    // `export { x } from 'mod'` — pure module re-export,
                    // no local name references. Hoist.
                    hoist_spans.push(span);
                } else {
                    // `export { x, y }` (no `from`) — local name re-export.
                    // Drop.
                    drop_spans.push(span);
                }
            }
            Statement::ExportDefaultDeclaration(decl) => {
                // `export default <expr>` — drop. Expressions may reference
                // locals; we don't try to disambiguate. The default export
                // surface goes away but the body keeps type-checking.
                drop_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportAllDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            _ => {}
        }
    }

    if hoist_spans.is_empty() && strip_keyword_spans.is_empty() && drop_spans.is_empty() {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
        };
    }

    // Hoisted prelude: emit each hoist-span verbatim, joined by newlines.
    let mut hoisted = String::new();
    for &(start, end) in &hoist_spans {
        hoisted.push_str(&content[start..end]);
        if !content[start..end].ends_with('\n') {
            hoisted.push('\n');
        }
    }

    // Body with hoisted + strip-keyword + dropped regions all blanked.
    // For strip-keyword spans we only blank the keyword prefix, not the
    // declaration — the declaration stays at its original byte position
    // in the body, with the `export ` replaced by spaces.
    let mut blank_spans: Vec<(usize, usize)> =
        Vec::with_capacity(hoist_spans.len() + strip_keyword_spans.len() + drop_spans.len());
    blank_spans.extend(hoist_spans.iter().copied());
    blank_spans.extend(strip_keyword_spans.iter().copied());
    blank_spans.extend(drop_spans.iter().copied());
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
                let byte_len = ch.len_utf8();
                for _ in 0..byte_len {
                    body.push(' ');
                }
            }
        }
        cursor = end;
    }
    body.push_str(&content[cursor..]);

    SplitScript { hoisted, body }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_imports_or_exports_passes_through() {
        let s = split_imports("let x = 1;", ScriptLang::Js);
        assert_eq!(s.hoisted, "");
        assert_eq!(s.body, "let x = 1;");
    }

    #[test]
    fn single_import_is_hoisted() {
        let src = "import { writable } from 'svelte/store';\nlet x = 1;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(
            s.hoisted
                .contains("import { writable } from 'svelte/store';")
        );
        assert!(s.body.contains("let x = 1;"));
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
        assert!(s.hoisted.contains("import a from 'a';"));
        assert!(s.hoisted.contains("import b from 'b';"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn type_only_imports_hoisted() {
        let src = "import type { Foo } from './foo';\nlet x: Foo = bar;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("import type { Foo }"));
    }

    #[test]
    fn export_const_keyword_is_stripped_keeping_declaration_in_body() {
        // The declaration body is what we care about for type-checking.
        // The `export ` prefix is blanked but `const PI = 3.14;` stays
        // at its original position in the body.
        let src = "let x = 1;\nexport const PI = 3.14;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(
            !s.hoisted.contains("export"),
            "should not hoist:\n{}",
            s.hoisted
        );
        assert!(
            !s.body.contains("export"),
            "should be blanked from body:\n{}",
            s.body
        );
        assert!(
            s.body.contains("const PI = 3.14;"),
            "declaration must survive:\n{}",
            s.body
        );
    }

    #[test]
    fn export_function_keyword_is_stripped() {
        // Svelte 5 component-level method export. Keyword stripped so
        // the function body's references (which may use other locals)
        // stay in scope.
        let src = "let x = $state(0);\nexport function foo() { return x; }";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(!s.hoisted.contains("export"));
        assert!(
            s.body.contains("function foo()"),
            "function declaration kept:\n{}",
            s.body
        );
        assert!(s.body.contains("let x = $state(0);"));
    }

    #[test]
    fn re_export_list_without_source_is_dropped_not_hoisted() {
        // `export { a, b }` (no `from` clause) re-exports local names.
        // Hoisting it to module level would fire TS2304/TS2552 because
        // `a` and `b` live inside $$render. We drop it entirely; the
        // declarations themselves stay intact in the body.
        let src = "let a = 1;\nlet b = 2;\nexport { a, b };";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(
            !s.hoisted.contains("export { a, b }"),
            "re-export without source should NOT be hoisted:\n{}",
            s.hoisted
        );
        assert!(
            !s.body.contains("export { a, b }"),
            "should be blanked from body"
        );
        assert!(s.body.contains("let a = 1;"));
        assert!(s.body.contains("let b = 2;"));
    }

    #[test]
    fn renamed_re_export_without_source_is_dropped() {
        let src = "let a = 1;\nexport { a as renamed };";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(!s.hoisted.contains("export"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn re_export_with_source_is_hoisted() {
        // `export { x } from 'mod'` doesn't reference local names — it's a
        // pure module-to-module re-export. Safe to hoist.
        let src = "export { foo } from './other';";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export { foo } from './other';"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn export_default_is_dropped() {
        // `export default x` could reference a local; we don't try to
        // disambiguate. Drop is safer than hoisting. Consumer-side
        // default-export surface goes away but body type-checks.
        let src = "let x = 1;\nexport default x;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(!s.hoisted.contains("export default"));
        assert!(!s.body.contains("export default"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn export_star_re_export_is_hoisted() {
        let src = "export * from './other';";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export * from './other';"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn body_offsets_preserved() {
        let src = "import a from 'a';\nlet x = 1;\nexport const y = 2;\nlet z = 3;";
        let original_let_z = src.find("let z").unwrap();
        let s = split_imports(src, ScriptLang::Ts);
        let new_let_z = s.body.find("let z").unwrap();
        assert_eq!(new_let_z, original_let_z);
    }

    #[test]
    fn newlines_preserved_inside_blanked_regions() {
        let src = "\
import {
    a,
    b,
} from 'mod';
let x = 1;
";
        let original_x_line = src.lines().position(|l| l.contains("let x")).unwrap();
        let s = split_imports(src, ScriptLang::Ts);
        let new_x_line = s.body.lines().position(|l| l.contains("let x")).unwrap();
        assert_eq!(new_x_line, original_x_line);
    }

    #[test]
    fn malformed_script_falls_back_to_passthrough() {
        let src = "import {{{ unbalanced";
        let s = split_imports(src, ScriptLang::Ts);
        let total = format!("{}{}", s.hoisted, s.body);
        assert!(total.contains("import"));
    }

    #[test]
    fn import_and_export_in_same_script() {
        // Import gets hoisted; bare re-export gets dropped (its name lives
        // inside $$render).
        let src = "\
import { writable } from 'svelte/store';
let count = writable(0);
export { count };
";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("import { writable }"));
        assert!(!s.hoisted.contains("export { count }"));
        assert!(s.body.contains("let count = writable(0);"));
    }
}
