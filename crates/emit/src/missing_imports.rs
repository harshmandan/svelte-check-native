//! Detection of imported `.svelte` modules that don't resolve on disk.
//!
//! The default `svelte-check` engine (the language-server / `svelte2tsx`
//! path) resolves every `.svelte` import against the real filesystem
//! through its own module-resolution host and reports `TS2307` when the
//! target file is missing. The tsgo engine we drive can't: `svelte`'s own
//! `types/index.d.ts` ships a `declare module '*.svelte'` wildcard, and
//! under tsgo's file-based resolution that wildcard swallows any
//! unresolved `.svelte` specifier — so tsgo (and `svelte-check --tsgo`)
//! never fires the error. We recover parity with the default
//! `svelte-check` by detecting the missing import ourselves and emitting
//! the same `TS2307`.
//!
//! Scope: RELATIVE specifiers (`./x.svelte`, `../x.svelte`) only. Their
//! resolution is unambiguous — relative to the importing file's directory
//! — so a missing target can never be a false positive. Aliased
//! (`$lib/...`) and bare (`some-lib/...`) `.svelte` specifiers depend on
//! `tsconfig` `paths` / `node_modules` resolution and are left to tsgo
//! (currently swallowed by the wildcard, matching `--tsgo`).

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use svn_parser::{Document, parse_script_body};

/// A relative `.svelte` module specifier imported (or re-exported) by a
/// component, carrying the byte span of its string literal — including
/// the surrounding quotes — in the ORIGINAL `.svelte` source.
pub struct SvelteImportRef {
    /// The specifier exactly as written, e.g. `./CardErrorState.svelte`.
    pub specifier: String,
    /// Byte offset of the string literal's opening quote in the original
    /// `.svelte` source.
    pub start: u32,
    /// Byte offset one past the closing quote, in the original source.
    pub end: u32,
}

/// Collect every relative `.svelte` specifier imported or re-exported at
/// the top level of the instance and module scripts.
///
/// Covers `import … from '…'`, `export … from '…'`, and `export * from
/// '…'` — every statement form that carries a module specifier. Type-only
/// imports are included: `import type X from './Missing.svelte'` fires
/// `TS2307` upstream just like a value import.
pub fn collect_relative_svelte_imports(doc: &Document<'_>) -> Vec<SvelteImportRef> {
    let mut out = Vec::new();
    for section in [doc.instance_script.as_ref(), doc.module_script.as_ref()]
        .into_iter()
        .flatten()
    {
        let allocator = Allocator::default();
        let parsed = parse_script_body(&allocator, section.content, section.lang);
        if parsed.panicked {
            // A syntactically broken script yields a garbage AST; the
            // caller reports the parse error separately. Skip.
            continue;
        }
        // oxc spans are relative to the script `content`; the section's
        // `content_range.start` translates them back to the full source.
        let base = section.content_range.start;
        for stmt in &parsed.program.body {
            let literal = match stmt {
                Statement::ImportDeclaration(decl) => Some(&decl.source),
                Statement::ExportNamedDeclaration(decl) => decl.source.as_ref(),
                Statement::ExportAllDeclaration(decl) => Some(&decl.source),
                _ => None,
            };
            if let Some(lit) = literal {
                let spec = lit.value.as_str();
                if is_relative_svelte(spec) {
                    out.push(SvelteImportRef {
                        specifier: spec.to_string(),
                        start: base + lit.span.start,
                        end: base + lit.span.end,
                    });
                }
            }
        }
    }
    out
}

/// A relative import that names a Svelte component file: begins with
/// `./` or `../` and ends in `.svelte`. Excludes `.svelte.ts` / `.svelte.js`
/// runes-module specifiers (those end in `.ts` / `.js`).
fn is_relative_svelte(spec: &str) -> bool {
    (spec.starts_with("./") || spec.starts_with("../")) && spec.ends_with(".svelte")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(src: &str) -> Vec<SvelteImportRef> {
        let (doc, _) = svn_parser::parse_sections(src);
        collect_relative_svelte_imports(&doc)
    }

    #[test]
    fn flags_relative_svelte_import() {
        let src = "<script lang=\"ts\">\nimport Foo from './Missing.svelte'\n</script>\n";
        let r = refs(src);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].specifier, "./Missing.svelte");
        // The recorded span points at the opening quote of the specifier.
        assert_eq!(
            &src[r[0].start as usize..r[0].end as usize],
            "'./Missing.svelte'"
        );
    }

    #[test]
    fn flags_parent_relative_and_export_from() {
        let src = "<script lang=\"ts\">\nexport { default } from '../a/B.svelte'\n</script>\n";
        let r = refs(src);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].specifier, "../a/B.svelte");
    }

    #[test]
    fn ignores_aliased_and_bare_and_runes() {
        let src = "<script lang=\"ts\">\n\
            import A from '$lib/A.svelte'\n\
            import B from 'some-lib/B.svelte'\n\
            import C from './C.svelte.ts'\n\
            import D from './D.ts'\n\
            </script>\n";
        assert!(refs(src).is_empty());
    }

    #[test]
    fn collects_from_module_script_too() {
        let src = "<script module lang=\"ts\">\nimport M from './M.svelte'\n</script>\n\
            <script lang=\"ts\">\nimport I from './I.svelte'\n</script>\n";
        let mut got: Vec<String> = refs(src).into_iter().map(|r| r.specifier).collect();
        got.sort();
        assert_eq!(
            got,
            vec!["./I.svelte".to_string(), "./M.svelte".to_string()]
        );
    }
}
