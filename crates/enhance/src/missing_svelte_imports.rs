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
    ExportAllDeclaration, ExportFromDeclaration, Expression, ImportDeclaration, ImportExpression,
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
        if any_wildcard_in_scope(workspace, tsconfig) {
            return Self { inner: None };
        }
        let options = ResolveOptions {
            tsconfig: Some(TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: tsconfig.to_path_buf(),
                // Never follow project references: Auto scopes paths
                // by the referenced project's directory, so a
                // composite reference sitting beside the entry config
                // (a common scaffold shape) covers the whole
                // workspace with its own — usually empty — paths and
                // shadows the entry config's aliases, inventing
                // TS2307s. TypeScript resolves a project's own
                // imports with that project's paths regardless of
                // references, and every check runs with its own
                // project's tsconfig here.
                references: TsconfigReferences::Disabled,
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
            Some(resolver) => {
                resolver.resolve(dir, specifier).is_ok()
                    || sidecar_specifier(specifier)
                        .is_some_and(|sidecar| resolver.resolve(dir, &sidecar).is_ok())
            }
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

    /// `export { Foo } from './Foo.svelte'`. This is its own node kind;
    /// a specifier list with no `from` clause (`ExportNamedDeclaration`)
    /// names no module and needs no visitor, and the declaration form
    /// (`ExportDeclaration`, e.g. `export const x =
    /// import('./y.svelte')`) is reached by the default walk.
    fn visit_export_from_declaration(&mut self, it: &ExportFromDeclaration<'a>) {
        self.record(&it.source);
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

/// The declaration-sidecar specifier for a `.svelte` specifier:
/// `x.svelte` → `x.d.svelte.ts`.
///
/// `allowArbitraryExtensions` lets a user satisfy `import
/// './Generated.svelte'` with a `Generated.d.svelte.ts` and no component
/// file at all — TypeScript's own documented mechanism (`x.css` looks up
/// `x.d.css.ts`), and the one our generated overlays rely on. The
/// resolver has no notion of it, so without this the import looked
/// unresolvable and we invented a TS2307 on code the compiler accepts.
///
/// The rewritten specifier is resolved through the SAME resolver as the
/// original, so the sidecar is found wherever the compiler would find
/// it: beside a relative target, behind a `tsconfig` `paths` alias, or
/// inside a package (node_modules + `exports`) — not just as a sibling
/// of a `./`-relative import.
fn sidecar_specifier(specifier: &str) -> Option<String> {
    let stem = specifier.strip_suffix(".svelte")?;
    Some(format!("{stem}.d.svelte.ts"))
}

/// A specifier that names a Svelte component file: ends in `.svelte`.
/// Excludes `.svelte.ts` / `.svelte.js` runes-module specifiers (those end
/// in `.ts` / `.js`). Query-suffixed forms (`x.svelte?raw`) are excluded —
/// they don't end in `.svelte` and aren't TS's concern.
fn is_svelte_specifier(spec: &str) -> bool {
    spec.ends_with(".svelte")
}

/// Whether a `declare module '*.svelte'` wildcard is live anywhere in
/// the program. When one is, the default `svelte-check` resolves every
/// `.svelte` import through it — it strips svelte's OWN wildcard but not
/// a user's — so we must not fire, or we invent an error the compiler
/// does not produce. See the module docs.
///
/// The question is "is a wildcard in the PROGRAM", not "is a wildcard
/// under this directory". Scanning a directory got that wrong in both
/// directions of the monorepo shape: a shared `.d.ts` sitting above the
/// workspace and pulled in by the tsconfig's own `include` was missed
/// entirely, so every `.svelte` import in that project reported a
/// TS2307 the user could do nothing about — they had already declared
/// the wildcard.
///
/// So: walk the directories the tsconfig actually admits (its
/// `include` roots and declared `typeRoots`, plus the workspace
/// itself) and scan its `files` entries individually — a lone ambient
/// `.d.ts` is commonly pulled in through `files` and may live outside
/// every include root. `node_modules` is still skipped wholesale —
/// svelte's own wildcard lives there and the default engine strips
/// exactly that one, so honouring it would suppress the check for
/// every project.
fn any_wildcard_in_scope(workspace: &Path, tsconfig: &Path) -> bool {
    let scope = tsconfig_scope(tsconfig);
    scope.files.iter().any(|f| {
        std::fs::read_to_string(f)
            .ok()
            .is_some_and(|src| declares_svelte_wildcard(&src))
    }) || scan_roots(workspace, scope.dirs)
        .iter()
        .any(|r| dir_declares_svelte_wildcard(r))
}

/// Collapse the workspace plus the include roots into a minimal set of
/// directories to walk. Containment is honoured in BOTH directions: a
/// new root inside an existing one is skipped (its subtree is walked
/// anyway), and a new root ABOVE an existing one replaces it — e.g. an
/// include of `..` subsumes the workspace itself, and keeping both
/// would re-traverse the whole workspace subtree a second time.
fn scan_roots(workspace: &Path, include_roots: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut roots: Vec<PathBuf> = vec![workspace.to_path_buf()];
    for root in include_roots {
        if roots.iter().any(|r| root.starts_with(r)) {
            continue;
        }
        roots.retain(|r| !r.starts_with(&root));
        roots.push(root);
    }
    roots
}

/// What a tsconfig admits into the program beyond the workspace
/// subtree: directories to walk and individual files to scan.
struct TsconfigScope {
    dirs: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

/// Derive the scan scope from the tsconfig chain.
///
/// Directories: the leading literal segments of each `include` pattern
/// (everything from the first wildcard segment on is dropped) —
/// relative patterns resolved against the winning config's directory,
/// absolute ones taken as-is — plus each declared `typeRoots` entry,
/// anchored on its declaring config the way TS rebases file-path
/// options. Files: each `files` entry, likewise anchored.
fn tsconfig_scope(tsconfig: &Path) -> TsconfigScope {
    let mut scope = TsconfigScope {
        dirs: Vec::new(),
        files: Vec::new(),
    };
    let Ok(chain) = svn_core::tsconfig::load_chain(tsconfig) else {
        return scope;
    };
    if let Some((winner, patterns)) =
        svn_core::tsconfig::winning_patterns(&chain, |f| f.include.as_deref())
    {
        let base = winner.config_dir();
        for pattern in patterns {
            let literal: Vec<&str> = pattern
                .split('/')
                .take_while(|segment| !segment.contains('*') && !segment.contains('?'))
                .collect();
            let literal = literal.join("/");
            let candidate = Path::new(&literal);
            // An absolute pattern is already anchored at the filesystem
            // root; gluing it under the config dir would fabricate a
            // path that exists nowhere and get the root silently
            // dropped.
            let dir = if candidate.is_absolute() {
                candidate.to_path_buf()
            } else {
                base.join(candidate)
            };
            let dir = normalise(&dir);
            if dir.is_dir() && !scope.dirs.contains(&dir) {
                scope.dirs.push(dir);
            }
        }
    }
    if let Some((winner, roots)) =
        svn_core::tsconfig::winning_field(&chain, |f| f.compiler_options.type_roots.as_deref())
    {
        let base = winner.config_dir();
        for root in roots {
            let dir = normalise(&base.join(root));
            if dir.is_dir() && !scope.dirs.contains(&dir) {
                scope.dirs.push(dir);
            }
        }
    }
    if let Some((winner, entries)) =
        svn_core::tsconfig::winning_patterns(&chain, |f| f.files.as_deref())
    {
        let base = winner.config_dir();
        for entry in entries {
            let file = normalise(&base.join(entry));
            if file.is_file() && !scope.files.contains(&file) {
                scope.files.push(file);
            }
        }
    }
    scope
}

/// `..`/`.` collapsing without touching the filesystem.
fn normalise(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            std::path::Component::ParentDir => {
                out.pop();
            }
            std::path::Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Does any `.d.ts` under `dir` declare the wildcard?
fn dir_declares_svelte_wildcard(dir: &Path) -> bool {
    let skip = |name: &str| {
        matches!(
            name,
            "node_modules" | ".svelte-kit" | ".git" | ".cache" | ".svelte-check"
        )
    };
    walkdir::WalkDir::new(dir)
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
    /// A wildcard in a shared `.d.ts` ABOVE the workspace, pulled into
    /// the program by the tsconfig's own `include`, still counts.
    ///
    /// Scanning only the workspace subtree missed it, and every
    /// `.svelte` import in that project then reported a TS2307 the user
    /// could do nothing about — they had already declared the wildcard.
    #[test]
    fn wildcard_above_the_workspace_is_seen_when_the_tsconfig_includes_it() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("shared")).unwrap();
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(
            root.join("shared/global.d.ts"),
            "declare module '*.svelte' { const c: any; export default c; }",
        )
        .unwrap();
        let tsconfig = root.join("app/tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "include": ["src/**/*", "../shared/**/*"] }"#,
        )
        .unwrap();

        assert!(
            any_wildcard_in_scope(&root.join("app"), &tsconfig),
            "wildcard reachable through the tsconfig include was not seen"
        );
    }

    /// A wildcard pulled in solely through the tsconfig's `files`
    /// array — reachable through no include root — still lives in the
    /// program, so it must disable the check.
    #[test]
    fn wildcard_in_files_entry_outside_include_roots_is_seen() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("shared")).unwrap();
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(
            root.join("shared/svelte-shim.d.ts"),
            "declare module '*.svelte' { const c: any; export default c; }",
        )
        .unwrap();
        let tsconfig = root.join("app/tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "include": ["src/**/*"], "files": ["../shared/svelte-shim.d.ts"] }"#,
        )
        .unwrap();

        assert!(
            any_wildcard_in_scope(&root.join("app"), &tsconfig),
            "wildcard reachable only through the files array was not seen"
        );
    }

    /// Same for a wildcard living under a declared typeRoot outside
    /// the workspace and every include root.
    #[test]
    fn wildcard_under_declared_type_root_is_seen() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("typings/shims")).unwrap();
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        std::fs::write(
            root.join("typings/shims/index.d.ts"),
            "declare module '*.svelte' { const c: any; export default c; }",
        )
        .unwrap();
        let tsconfig = root.join("app/tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "include": ["src/**/*"], "compilerOptions": { "typeRoots": ["../typings"] } }"#,
        )
        .unwrap();

        assert!(
            any_wildcard_in_scope(&root.join("app"), &tsconfig),
            "wildcard under a declared typeRoot was not seen"
        );
    }

    /// The guard must not over-suppress: with no wildcard anywhere in
    /// scope the check has to stay live, or the whole layer goes quiet.
    #[test]
    fn no_wildcard_in_scope_leaves_the_check_live() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("app/src")).unwrap();
        let tsconfig = root.join("app/tsconfig.json");
        std::fs::write(&tsconfig, r#"{ "include": ["src/**/*"] }"#).unwrap();

        assert!(!any_wildcard_in_scope(&root.join("app"), &tsconfig));
    }

    #[test]
    fn sidecar_specifier_rewrites_only_svelte() {
        assert_eq!(
            sidecar_specifier("./Gen.svelte").as_deref(),
            Some("./Gen.d.svelte.ts")
        );
        assert_eq!(
            sidecar_specifier("$gen/Gen.svelte").as_deref(),
            Some("$gen/Gen.d.svelte.ts")
        );
        assert_eq!(sidecar_specifier("./Gen.svelte.ts"), None);
        assert_eq!(sidecar_specifier("pkg"), None);
    }

    /// `allowArbitraryExtensions` lets a hand-written
    /// `<name>.d.svelte.ts` satisfy a `.svelte` import with no component
    /// file present. The sidecar is looked up through the same resolver
    /// as the import itself, so it works wherever the compiler would
    /// find it — beside a relative target AND behind a `paths` alias.
    #[test]
    fn declaration_sidecar_satisfies_relative_and_aliased_specifiers() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src/gen")).unwrap();
        std::fs::write(
            root.join("src/gen/Gen.d.svelte.ts"),
            "declare const c: unknown; export default c;",
        )
        .unwrap();
        let tsconfig = root.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "paths": { "$gen/*": ["./src/gen/*"] } } }"#,
        )
        .unwrap();

        let resolver = SvelteImportResolver::new(root, &tsconfig);
        let src = root.join("src");
        assert!(resolver.resolves(&src, "./gen/Gen.svelte"));
        assert!(resolver.resolves(&src, "$gen/Gen.svelte"));
        assert!(!resolver.resolves(&src, "./gen/Absent.svelte"));
        assert!(!resolver.resolves(&src, "$gen/Absent.svelte"));
    }

    /// An absolute `include` pattern is anchored at the filesystem
    /// root. Resolving it against the config dir fabricated
    /// `<configdir>/abs/...`, which failed the directory probe — so a
    /// wildcard reachable only through the absolute include stayed
    /// invisible and the ambient guard never disabled the check.
    #[test]
    fn absolute_include_pattern_reaches_its_wildcard() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("shared")).unwrap();
        std::fs::create_dir_all(root.join("app")).unwrap();
        std::fs::write(
            root.join("shared/global.d.ts"),
            "declare module '*.svelte' { const c: any; export default c; }",
        )
        .unwrap();
        let tsconfig = root.join("app/tsconfig.json");
        let abs_pattern = root.join("shared").join("**/*");
        std::fs::write(
            &tsconfig,
            format!(r#"{{ "include": ["{}"] }}"#, abs_pattern.display()),
        )
        .unwrap();

        assert!(
            any_wildcard_in_scope(&root.join("app"), &tsconfig),
            "wildcard behind an absolute include pattern was not seen"
        );
    }

    /// Root dedup is bidirectional: a root above the workspace (e.g.
    /// an include of `..`) subsumes it, so the workspace subtree must
    /// not be kept as a second root and walked twice.
    #[test]
    fn scan_roots_collapses_containment_both_ways() {
        let ws = PathBuf::from("/repo/app");
        // Below the workspace: contributes nothing.
        assert_eq!(
            scan_roots(&ws, vec![PathBuf::from("/repo/app/src")]),
            vec![ws.clone()]
        );
        // Above the workspace: replaces it.
        assert_eq!(
            scan_roots(&ws, vec![PathBuf::from("/repo")]),
            vec![PathBuf::from("/repo")]
        );
        // Disjoint: both kept.
        assert_eq!(
            scan_roots(&ws, vec![PathBuf::from("/shared")]),
            vec![ws.clone(), PathBuf::from("/shared")]
        );
        // A later, wider root also swallows earlier include roots.
        assert_eq!(
            scan_roots(
                &ws,
                vec![PathBuf::from("/repo/shared"), PathBuf::from("/repo")]
            ),
            vec![PathBuf::from("/repo")]
        );
    }
}
