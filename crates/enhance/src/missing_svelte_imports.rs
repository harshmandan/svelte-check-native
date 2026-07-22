//! Detection of imported `.svelte` modules that don't resolve on disk.
//!
//! The default `svelte-check` engine resolves every `.svelte` import
//! against the real filesystem through its own module-resolution host and
//! reports `TS2307` when the target file is missing. The tsgo engine we
//! drive can't: svelte's own `types/index.d.ts` ships a
//! `declare module '*.svelte'` wildcard, and under tsgo's file-based
//! resolution that wildcard resolves any unresolved `.svelte` specifier to
//! `any` — so tsgo (and `svelte-check --tsgo`) never fires the error. We
//! recover parity with the default `svelte-check` by detecting the missing
//! import ourselves and emitting the same `TS2307`.
//!
//! Scope: RELATIVE specifiers (`./x.svelte`, `../x.svelte`) only. Their
//! resolution is unambiguous — relative to the importing file's directory
//! — so a missing target can never be a false positive. Aliased
//! (`$lib/...`) and bare (`some-lib/...`) `.svelte` specifiers depend on
//! `tsconfig` `paths` / `node_modules` resolution and are handled
//! elsewhere.

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::Statement;
use svn_core::{PositionMap, Range};
use svn_parser::{Document, parse_script_body};

/// A position-mapped enhancement diagnostic, ready for the caller to lift
/// into its own diagnostic type. Positions are 1-based (line and column),
/// matching the CLI's `CheckDiagnostic`; `code` is the TS numeric code.
pub struct EnhancementDiagnostic {
    pub file: PathBuf,
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub code: u32,
    pub message: String,
}

/// Produce a `TS2307` for every relative `.svelte` import in `doc` whose
/// target file is missing on disk.
///
/// Fires only when a relative specifier resolves to a path where NEITHER
/// the `.svelte` component NOR any extension-appended sibling TS's module
/// resolution would accept exists — so it can never false-positive on a
/// resolvable import.
pub fn missing_svelte_import_diagnostics(
    file: &Path,
    source: &str,
    doc: &Document<'_>,
) -> Vec<EnhancementDiagnostic> {
    let refs = collect_relative_svelte_imports(doc);
    if refs.is_empty() {
        return Vec::new();
    }
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let pm = PositionMap::new(source);
    refs.into_iter()
        .filter(|r| !svelte_import_resolves(&dir.join(&r.specifier)))
        .map(|r| {
            let (start, end) = pm.range_positions(Range::new(r.start, r.end));
            EnhancementDiagnostic {
                file: file.to_path_buf(),
                // PositionMap is 0-based; the caller's diagnostic is 1-based.
                line: start.line.saturating_add(1),
                column: start.character.saturating_add(1),
                end_line: end.line.saturating_add(1),
                end_column: end.character.saturating_add(1),
                code: 2307,
                message: format!(
                    "Cannot find module '{}' or its corresponding type declarations.",
                    r.specifier
                ),
            }
        })
        .collect()
}

/// A relative `.svelte` module specifier imported (or re-exported) by a
/// component, carrying the byte span of its string literal — including the
/// surrounding quotes — in the ORIGINAL `.svelte` source.
struct SvelteImportRef {
    specifier: String,
    start: u32,
    end: u32,
}

/// Collect every relative `.svelte` specifier imported or re-exported at
/// the top level of the instance and module scripts.
///
/// Covers `import … from '…'`, `export … from '…'`, and `export * from
/// '…'` — every statement form that carries a module specifier. Type-only
/// imports are included: `import type X from './Missing.svelte'` fires
/// `TS2307` upstream just like a value import.
fn collect_relative_svelte_imports(doc: &Document<'_>) -> Vec<SvelteImportRef> {
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

/// A relative import that names a Svelte component file: begins with `./`
/// or `../` and ends in `.svelte`. Excludes `.svelte.ts` / `.svelte.js`
/// runes-module specifiers (those end in `.ts` / `.js`).
fn is_relative_svelte(spec: &str) -> bool {
    (spec.starts_with("./") || spec.starts_with("../")) && spec.ends_with(".svelte")
}

/// Whether a relative `.svelte` specifier resolving to `base` (a
/// `…/Foo.svelte` path) is satisfiable by any file TS's module resolution
/// would accept: the component itself, an extension-appended sibling (a
/// `Foo.svelte.ts` runes module, a `.d.ts`, a plain JS pairing), or a
/// directory index. Only when none exist is the import genuinely missing.
fn svelte_import_resolves(base: &Path) -> bool {
    // The component file itself. `is_file` (not `exists`): a *directory*
    // named `Foo.svelte` doesn't satisfy `./Foo.svelte` unless it has an
    // index (handled next) — TS would report TS2307 for an empty dir.
    if base.is_file() {
        return true;
    }
    // Directory-index resolution: `./Foo.svelte/index.{ts,…}`. Rare (a dir
    // literally named `*.svelte`), but TS resolves it, so matching avoids a
    // false positive.
    if base.is_dir()
        && [
            "index.ts",
            "index.tsx",
            "index.d.ts",
            "index.js",
            "index.jsx",
        ]
        .iter()
        .any(|idx| base.join(idx).is_file())
    {
        return true;
    }
    // Under bundler / `allowArbitraryExtensions` resolution TS appends each
    // of these to `./Foo.svelte` before giving up.
    let stem = base.as_os_str();
    [".ts", ".tsx", ".d.ts", ".js", ".jsx"].iter().any(|ext| {
        let mut p = stem.to_os_string();
        p.push(ext);
        Path::new(&p).is_file()
    })
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
