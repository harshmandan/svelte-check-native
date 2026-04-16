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
pub fn split_imports(content: &str, lang: ScriptLang) -> SplitScript {
    // Fast path: no import/export keyword at all → nothing to hoist.
    if !content.contains("import") && !content.contains("export") {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
        };
    }

    let allocator = Allocator::default();
    let parsed = parse_script_body(&allocator, content, lang);

    if parsed.panicked {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
        };
    }

    let mut hoist_spans: Vec<(usize, usize)> = Vec::new();
    for stmt in &parsed.program.body {
        match stmt {
            Statement::ImportDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportNamedDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportDefaultDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            Statement::ExportAllDeclaration(decl) => {
                hoist_spans.push((decl.span.start as usize, decl.span.end as usize));
            }
            _ => {}
        }
    }

    if hoist_spans.is_empty() {
        return SplitScript {
            hoisted: String::new(),
            body: content.to_string(),
        };
    }

    // Hoisted prelude: emit each statement verbatim, joined by newlines.
    let mut hoisted = String::new();
    for &(start, end) in &hoist_spans {
        hoisted.push_str(&content[start..end]);
        if !content[start..end].ends_with('\n') {
            hoisted.push('\n');
        }
    }

    // Body with hoisted regions blanked. Replacing each span with ASCII
    // whitespace of the same byte length preserves byte offsets for the
    // source-map mapping that runs later.
    let mut body = String::with_capacity(content.len());
    let mut cursor = 0;
    for &(start, end) in &hoist_spans {
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
    fn export_const_is_hoisted() {
        let src = "let x = 1;\nexport const PI = 3.14;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export const PI = 3.14;"));
        assert!(!s.body.contains("export"));
        assert!(s.body.contains("let x = 1;"));
    }

    #[test]
    fn export_function_is_hoisted() {
        // Svelte 5 component-level method export.
        let src = "let x = $state(0);\nexport function foo() { return x; }";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export function foo()"));
        assert!(s.body.contains("let x = $state(0);"));
    }

    #[test]
    fn export_re_export_list_is_hoisted() {
        let src = "let a = 1;\nlet b = 2;\nexport { a, b };";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export { a, b };"));
        assert!(!s.body.contains("export"));
    }

    #[test]
    fn export_renamed_re_export_is_hoisted() {
        let src = "let a = 1;\nexport { a as renamed };";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export { a as renamed };"));
    }

    #[test]
    fn export_default_is_hoisted() {
        let src = "let x = 1;\nexport default x;";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("export default x;"));
        assert!(!s.body.contains("export"));
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
        let src = "\
import { writable } from 'svelte/store';
let count = writable(0);
export { count };
";
        let s = split_imports(src, ScriptLang::Ts);
        assert!(s.hoisted.contains("import { writable }"));
        assert!(s.hoisted.contains("export { count };"));
        assert!(s.body.contains("let count = writable(0);"));
    }
}
