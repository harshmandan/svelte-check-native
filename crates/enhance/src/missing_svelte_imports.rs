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
//! ## Resolution
//!
//! We resolve every `.svelte` specifier — relative (`./x.svelte`), aliased
//! (`$lib/x.svelte` via `tsconfig` `paths`), and bare
//! (`some-lib/x.svelte` via node_modules + package.json `exports`) — with
//! [`oxc_resolver`], the same bundler-grade resolver used across the oxc
//! ecosystem. It performs the real TS/node resolution tsgo would, so a
//! specifier fires `TS2307` only when genuinely unresolvable on disk. Only
//! specifiers ending in `.svelte` are considered (a `Foo.svelte.ts` runes
//! module ends in `.ts`, so it's excluded from collection and, when it
//! exists as a sibling, satisfies `./Foo.svelte` through the resolver's
//! extension list — matching TS).
//!
//! ## The ambient guard
//!
//! `oxc_resolver` is filesystem-only: it cannot see TypeScript ambient
//! `declare module` declarations. The default `svelte-check` strips
//! svelte's OWN `*.svelte` wildcard but keeps a USER-authored one, so a
//! project that declares its own `declare module '*.svelte'` resolves
//! every `.svelte` import and reports nothing. To avoid firing a false
//! positive there, [`SvelteImportResolver::new`] scans the workspace for a
//! user `*.svelte` wildcard and, if it finds one, disables the check
//! entirely (the resolver holds `None`).

use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{
    ExportAllDeclaration, ExportNamedDeclaration, Expression, ImportDeclaration, ImportExpression,
    StringLiteral,
};
use oxc_ast_visit::{Visit, walk};
use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
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

/// A shared `.svelte`-import resolver, built once per run and passed by
/// reference into the per-file pass (the inner resolver is thread-safe).
///
/// Holds `None` — disabling the whole check — when the workspace declares
/// its own `declare module '*.svelte'` wildcard (see the module docs' "The
/// ambient guard").
pub struct SvelteImportResolver {
    inner: Option<Resolver>,
}

impl SvelteImportResolver {
    /// A resolver that reports nothing — for the `--disable-enhance` flag /
    /// `SVN_DISABLE_ENHANCE` env kill-switch. Skips even the workspace scan.
    pub fn disabled() -> Self {
        Self { inner: None }
    }

    /// Build the resolver from the user's `tsconfig` (for `paths`/`baseUrl`)
    /// and workspace (for the ambient-wildcard guard).
    pub fn new(workspace: &Path, tsconfig: &Path) -> Self {
        if workspace_declares_svelte_wildcard(workspace) {
            return Self { inner: None };
        }
        let options = ResolveOptions {
            tsconfig: Some(TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: tsconfig.to_path_buf(),
                references: TsconfigReferences::Auto,
            })),
            // `.svelte` resolves the component file; the TS extensions let
            // `./Foo.svelte` also resolve a `Foo.svelte.ts` runes-module
            // sibling (`Foo.svelte` + `.ts`), matching TS's own append.
            extensions: [
                ".svelte",
                ".ts",
                ".tsx",
                ".d.ts",
                ".js",
                ".jsx",
                ".svelte.ts",
                ".svelte.js",
            ]
            .iter()
            .map(|s| (*s).to_string())
            .collect(),
            // `svelte` first: component libraries expose their `.svelte`
            // entry points under the `svelte` export condition.
            condition_names: ["svelte", "types", "import", "default"]
                .iter()
                .map(|s| (*s).to_string())
                .collect(),
            ..ResolveOptions::default()
        };
        Self {
            inner: Some(Resolver::new(options)),
        }
    }

    /// `true` if the given specifier resolves to a real file from `dir`.
    /// A disabled resolver (ambient guard tripped) reports everything as
    /// resolvable, so nothing fires.
    fn resolves(&self, dir: &Path, specifier: &str) -> bool {
        match &self.inner {
            None => true,
            Some(resolver) => resolver.resolve(dir, specifier).is_ok(),
        }
    }
}

/// Produce a `TS2307` for every `.svelte` import in `doc` whose target
/// module doesn't resolve on disk — via `oxc_resolver`, so relative,
/// aliased, and bare specifiers are all covered faithfully. Fires only on
/// a genuine resolution failure, so it can't false-positive on a
/// resolvable import.
pub fn missing_svelte_import_diagnostics(
    file: &Path,
    source: &str,
    doc: &Document<'_>,
    resolver: &SvelteImportResolver,
) -> Vec<EnhancementDiagnostic> {
    if resolver.inner.is_none() {
        return Vec::new();
    }
    let refs = collect_svelte_imports(doc);
    if refs.is_empty() {
        return Vec::new();
    }
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let pm = PositionMap::new(source);
    refs.into_iter()
        .filter(|r| !resolver.resolves(dir, &r.specifier))
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

/// A `.svelte` module specifier imported (or re-exported) by a component,
/// carrying the byte span of its string literal — including the
/// surrounding quotes — in the ORIGINAL `.svelte` source.
struct SvelteImportRef {
    specifier: String,
    start: u32,
    end: u32,
}

/// Collect every `.svelte` specifier imported or re-exported anywhere in
/// the instance and module scripts.
///
/// Covers static `import … from '…'`, `export … from '…'`, `export * from
/// '…'`, AND dynamic `import('…')` (which can appear nested in any
/// expression, so an AST walk — not a top-level statement scan — is
/// required). Type-only imports are included: `import type X from
/// './Missing.svelte'` fires `TS2307` upstream just like a value import.
fn collect_svelte_imports(doc: &Document<'_>) -> Vec<SvelteImportRef> {
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
        let mut collector = ImportCollector {
            base: section.content_range.start,
            out: &mut out,
        };
        collector.visit_program(&parsed.program);
    }
    out
}

/// AST visitor that records every `.svelte` module specifier — static and
/// dynamic — with its byte span translated back to the original source.
struct ImportCollector<'o> {
    /// `content_range.start` of the script section this program came from.
    base: u32,
    out: &'o mut Vec<SvelteImportRef>,
}

impl ImportCollector<'_> {
    fn record(&mut self, lit: &StringLiteral<'_>) {
        let spec = lit.value.as_str();
        if is_svelte_specifier(spec) {
            self.out.push(SvelteImportRef {
                specifier: spec.to_string(),
                start: self.base + lit.span.start,
                end: self.base + lit.span.end,
            });
        }
    }
}

impl<'a> Visit<'a> for ImportCollector<'_> {
    fn visit_import_declaration(&mut self, it: &ImportDeclaration<'a>) {
        self.record(&it.source);
    }

    fn visit_export_named_declaration(&mut self, it: &ExportNamedDeclaration<'a>) {
        if let Some(source) = &it.source {
            self.record(source);
        }
        // Recurse: `export const x = import('./y.svelte')` carries a
        // dynamic import inside the declaration.
        walk::walk_export_named_declaration(self, it);
    }

    fn visit_export_all_declaration(&mut self, it: &ExportAllDeclaration<'a>) {
        self.record(&it.source);
    }

    fn visit_import_expression(&mut self, it: &ImportExpression<'a>) {
        if let Expression::StringLiteral(lit) = &it.source {
            self.record(lit);
        }
        walk::walk_import_expression(self, it);
    }
}

/// A specifier that names a Svelte component file: ends in `.svelte`.
/// Excludes `.svelte.ts` / `.svelte.js` runes-module specifiers (those end
/// in `.ts` / `.js`). Query-suffixed forms (`x.svelte?raw`) are excluded —
/// they don't end in `.svelte` and aren't TS's concern.
fn is_svelte_specifier(spec: &str) -> bool {
    spec.ends_with(".svelte")
}

/// Whether the workspace declares its own `declare module '*.svelte'`
/// wildcard. When it does, the default `svelte-check` resolves every
/// `.svelte` import through it (it strips svelte's OWN wildcard but not the
/// user's), so we must not fire — see the module docs.
///
/// Walks the workspace, skipping `node_modules` (svelte's own wildcard
/// lives there and the default engine strips it), the generated
/// `.svelte-kit`, our cache, and VCS dirs. Only `.d.ts` files are read.
fn workspace_declares_svelte_wildcard(workspace: &Path) -> bool {
    let skip = |name: &str| {
        matches!(
            name,
            "node_modules" | ".svelte-kit" | ".git" | ".cache" | ".svelte-check"
        )
    };
    walkdir::WalkDir::new(workspace)
        .into_iter()
        .filter_entry(|e| !(e.file_type().is_dir() && e.file_name().to_str().is_some_and(skip)))
        .filter_map(Result::ok)
        .filter(|e| {
            e.file_type().is_file() && e.file_name().to_str().is_some_and(|n| n.ends_with(".d.ts"))
        })
        .any(|e| {
            std::fs::read_to_string(e.path())
                .ok()
                .is_some_and(|src| declares_svelte_wildcard(&src))
        })
}

/// Cheap textual check for `declare module '*.svelte'` / `"*.svelte"`. A
/// coarse scan is acceptable: the guard only needs to know the wildcard is
/// present, and a false hit merely suppresses our check (never a false
/// positive).
fn declares_svelte_wildcard(src: &str) -> bool {
    src.contains("declare module") && (src.contains("'*.svelte'") || src.contains("\"*.svelte\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refs(src: &str) -> Vec<SvelteImportRef> {
        let (doc, _) = svn_parser::parse_sections(src);
        collect_svelte_imports(&doc)
    }

    fn specifiers(src: &str) -> Vec<String> {
        let mut v: Vec<String> = refs(src).into_iter().map(|r| r.specifier).collect();
        v.sort();
        v
    }

    #[test]
    fn collects_relative_aliased_and_bare_svelte() {
        let src = "<script lang=\"ts\">\n\
            import R from './Rel.svelte'\n\
            import L from '$lib/Lib.svelte'\n\
            import B from 'some-lib/Bare.svelte'\n\
            </script>\n";
        assert_eq!(
            specifiers(src),
            vec![
                "$lib/Lib.svelte".to_string(),
                "./Rel.svelte".to_string(),
                "some-lib/Bare.svelte".to_string(),
            ]
        );
    }

    #[test]
    fn span_points_at_opening_quote() {
        let src = "<script lang=\"ts\">\nimport Foo from './Missing.svelte'\n</script>\n";
        let r = refs(src);
        assert_eq!(r.len(), 1);
        assert_eq!(
            &src[r[0].start as usize..r[0].end as usize],
            "'./Missing.svelte'"
        );
    }

    #[test]
    fn excludes_runes_modules_and_non_svelte() {
        let src = "<script lang=\"ts\">\n\
            import C from './C.svelte.ts'\n\
            import D from './D.ts'\n\
            import E from 'pkg'\n\
            </script>\n";
        assert!(refs(src).is_empty());
    }

    #[test]
    fn collects_dynamic_import() {
        // Dynamic import nested inside a function body — only an AST walk
        // (not a top-level statement scan) finds it.
        let src = "<script lang=\"ts\">\n\
            async function load() { return import('./Lazy.svelte') }\n\
            </script>\n";
        assert_eq!(specifiers(src), vec!["./Lazy.svelte".to_string()]);
    }

    #[test]
    fn collects_export_from_and_module_script() {
        let src = "<script module lang=\"ts\">\nexport * from './M.svelte'\n</script>\n\
            <script lang=\"ts\">\nexport { default } from './I.svelte'\n</script>\n";
        assert_eq!(
            specifiers(src),
            vec!["./I.svelte".to_string(), "./M.svelte".to_string()]
        );
    }

    #[test]
    fn wildcard_detection() {
        assert!(declares_svelte_wildcard(
            "declare module '*.svelte' { const c: any; export default c }"
        ));
        assert!(declares_svelte_wildcard(
            "declare module \"*.svelte\" { export default 1 }"
        ));
        assert!(!declares_svelte_wildcard("declare module '*.css' {}"));
        assert!(!declares_svelte_wildcard("import x from '*.svelte'"));
    }
}
