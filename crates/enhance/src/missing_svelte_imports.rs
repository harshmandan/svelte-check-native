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
//!
//! ## Plain `.ts` / `.js` importers
//!
//! The default engine reports the same `TS2307` in every TS/JS file of
//! the program, not just in components — `main.ts`, a kit `+page.ts`, a
//! `.svelte.ts` runes module. Those files are only reported when the
//! compiler type-checks them at all, so
//! [`missing_svelte_import_diagnostics_in_script`] applies TypeScript's
//! own rules (see [`script_is_type_checked`]): a TS file unless it opts
//! out with `// @ts-nocheck`; a JS file only under `checkJs` or a leading
//! `// @ts-check`; a declaration file only without `skipLibCheck`. A
//! component follows svelte-check's version of the same rule (see
//! [`component_is_type_checked`]). Every kind of reference counts, from
//! JSDoc import types to template expressions — see `svelte_refs`.
//!
//! ## Stubs: no wildcard fallout
//!
//! Reporting the `TS2307` is half the job. The default engine blanks
//! svelte's wildcard out of svelte's types when it loads them
//! (`DocumentSnapshot.fromNonSvelteFilePath` in the language server), so
//! a missing module stays unresolved: every use of it is the compiler's
//! error type and nothing else fires. Under tsgo the same import lands on
//! the wildcard, whose shape (a value-only `Component` default, no named
//! exports) fires errors the default engine never reports. So each
//! missing target also gets a stub ([`MISSING_MODULE_STUB`]): a
//! declaration file with no imports or exports, i.e. a script, not a
//! module. The overlay makes tsgo resolve the specifier to it (see
//! [`StubTarget`]); the compiler then reports TS2306 "is not a module" at
//! the specifier — which the pipeline drops, our TS2307 standing in its
//! place — and treats the import exactly as unresolved.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_parser::Parser;
use oxc_resolver::{
    ResolveOptions, Resolver, TsconfigDiscovery, TsconfigOptions, TsconfigReferences,
};
use oxc_span::{GetSpan, SourceType};
use svn_core::{PositionMap, Range};
use svn_parser::{Document, Fragment};

use crate::svelte_refs::{self, SvelteImportRef};

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

/// What the missing-import pass found in one file.
#[derive(Default)]
pub struct MissingImports {
    /// One `TS2307` per unresolvable `.svelte` reference.
    pub diagnostics: Vec<EnhancementDiagnostic>,
    /// The modules behind them — the input to [`module_stubs`].
    pub stub_targets: Vec<StubTarget>,
}

/// Where a missing module's stub has to be reachable from.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum StubTarget {
    /// A relative specifier: the absolute `.svelte` path it names. The
    /// stub goes where that component's generated declaration would, so
    /// `rootDirs` reaches it from every importer the way it reaches a
    /// real component's.
    SvelteFile(PathBuf),
    /// An aliased or bare specifier, exactly as written. `rootDirs` never
    /// applies to these, so the stub is reached through an exact-match
    /// `paths` entry, which wins over wildcard patterns like `$lib/*`.
    Specifier(String),
}

/// The stub every missing module resolves to. No `import` or `export`,
/// so the compiler loads it as a script: an import that resolves to it
/// draws TS2306 and is then unresolved, the way the default engine sees
/// a missing module.
pub const MISSING_MODULE_STUB: &str = "// generated by svelte-check-native; do not edit\n";

/// One stub per distinct target across every importer, sorted, so the
/// output is stable across runs.
pub fn module_stubs(
    targets: impl IntoIterator<Item = StubTarget>,
) -> Vec<(StubTarget, &'static str)> {
    targets
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .map(|target| (target, MISSING_MODULE_STUB))
        .collect()
}

/// A shared `.svelte`-import resolver, built once per run and passed by
/// reference into the per-file pass (the inner resolver is thread-safe).
///
/// Holds `None` — disabling the whole check — when the workspace declares
/// its own `declare module '*.svelte'` wildcard (see the module docs' "The
/// ambient guard").
pub struct SvelteImportResolver {
    inner: Option<Resolver>,
    /// The program's `checkJs`; `None` when the tsconfig chain leaves it
    /// unset, which TypeScript treats differently from `false` only for
    /// diagnostics this check never produces (see
    /// [`script_is_type_checked`]).
    check_js: Option<bool>,
    skip_lib_check: bool,
}

impl SvelteImportResolver {
    /// A resolver that reports nothing — for the `--disable-enhance` flag /
    /// `SVN_DISABLE_ENHANCE` env kill-switch. Skips even the workspace scan.
    pub fn disabled() -> Self {
        Self {
            inner: None,
            check_js: None,
            skip_lib_check: false,
        }
    }

    /// `false` when the check reports nothing — the kill-switch or the
    /// ambient guard — so a caller can skip reading files for it.
    pub fn is_enabled(&self) -> bool {
        self.inner.is_some()
    }

    /// Build the resolver from the user's `tsconfig` (for `paths`/`baseUrl`)
    /// and workspace (for the ambient-wildcard guard).
    pub fn new(workspace: &Path, tsconfig: &Path) -> Self {
        if any_wildcard_in_scope(workspace, tsconfig) {
            return Self::disabled();
        }
        let compiler_options = svn_core::tsconfig::load(tsconfig)
            .map(|config| config.compiler_options)
            .unwrap_or_default();
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
            check_js: compiler_options.check_js,
            skip_lib_check: compiler_options.skip_lib_check.unwrap_or(false),
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

/// Produce a `TS2307` for every `.svelte` reference in a component —
/// scripts and template — whose target module doesn't resolve on disk —
/// via `oxc_resolver`, so relative, aliased, and bare specifiers are all
/// covered faithfully. Fires only on a genuine resolution failure, so it
/// can't false-positive on a resolvable import, and only in a component
/// the compiler checks (see [`component_is_type_checked`]).
pub fn missing_svelte_import_diagnostics(
    file: &Path,
    source: &str,
    doc: &Document<'_>,
    fragment: &Fragment,
    resolver: &SvelteImportResolver,
) -> MissingImports {
    if resolver.inner.is_none() || !source.contains(".svelte") {
        return MissingImports::default();
    }
    let is_ts = svn_parser::is_ts_svelte(source);
    if !component_is_type_checked(doc, is_ts, resolver.check_js) {
        return MissingImports::default();
    }
    let refs = svelte_refs::collect_component(doc, fragment, source, is_ts);
    missing_imports(file, source, refs, resolver)
}

/// Whether svelte-check type-checks a component at all. It prepends the
/// `// @ts-check` / `// @ts-nocheck` its script opens with to the
/// generated code (see [`svn_parser::ts_check_directive`]), so that
/// directive decides; otherwise a TS component is checked and a
/// JavaScript one only under `checkJs: true`.
fn component_is_type_checked(doc: &Document<'_>, is_ts: bool, check_js: Option<bool>) -> bool {
    let script = doc.instance_script.as_ref().or(doc.module_script.as_ref());
    match script.and_then(|s| svn_parser::ts_check_directive(s.content)) {
        Some(directive) => directive == "@ts-check",
        None => is_ts || check_js == Some(true),
    }
}

/// [`missing_svelte_import_diagnostics`] for a plain TS/JS module of the
/// program (`main.ts`, a kit `+page.ts`, a `.svelte.ts` runes module):
/// the whole file is the script. Reports nothing for a file the compiler
/// does not type-check (see [`script_is_type_checked`]) or whose
/// extension is not a TS/JS one.
///
/// The caller decides which files belong to the program (`include` /
/// `exclude` / `files`, and `allowJs` for JS sources).
pub fn missing_svelte_import_diagnostics_in_script(
    file: &Path,
    source: &str,
    resolver: &SvelteImportResolver,
) -> MissingImports {
    // The early-outs skip the parse for the common file that imports no
    // component at all.
    if resolver.inner.is_none() || !source.contains(".svelte") {
        return MissingImports::default();
    }
    let Ok(source_type) = SourceType::from_path(file) else {
        return MissingImports::default();
    };
    if is_declaration_file(file) && resolver.skip_lib_check {
        return MissingImports::default();
    }
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.fatal_error
        || !script_is_type_checked(&parsed.program, source_type, resolver.check_js)
    {
        return MissingImports::default();
    }
    let refs = svelte_refs::collect_module(&parsed.program, source_type.is_javascript());
    missing_imports(file, source, refs, resolver)
}

/// Turn the collected specifiers of one file into `TS2307`s, and stub
/// requests, for those that resolve to nothing.
fn missing_imports(
    file: &Path,
    source: &str,
    refs: Vec<SvelteImportRef>,
    resolver: &SvelteImportResolver,
) -> MissingImports {
    if refs.is_empty() {
        return MissingImports::default();
    }
    let dir = file.parent().unwrap_or_else(|| Path::new("."));
    let pm = PositionMap::new(source);
    let mut found = MissingImports::default();
    for r in refs {
        if resolver.resolves(dir, &r.specifier) {
            continue;
        }
        let (start, end) = pm.range_positions(Range::new(r.start, r.end));
        found.diagnostics.push(EnhancementDiagnostic {
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
        });
        found
            .stub_targets
            .push(if is_relative_specifier(&r.specifier) {
                StubTarget::SvelteFile(normalise(&dir.join(&r.specifier)))
            } else {
                StubTarget::Specifier(r.specifier)
            });
    }
    found
}

/// TypeScript's `isExternalModuleNameRelative`: `./` / `../` (or exactly
/// `.` / `..`) and rooted paths. Path mapping never applies to these.
fn is_relative_specifier(specifier: &str) -> bool {
    specifier == "."
        || specifier == ".."
        || specifier.starts_with("./")
        || specifier.starts_with("../")
        || Path::new(specifier).is_absolute()
}

/// Whether TypeScript reports semantic errors for this file at all.
///
/// Mirrors the compiler's own gate: a `// @ts-nocheck` / `// @ts-check`
/// directive decides when present (the later one wins), otherwise a TS
/// file is checked and a JS file only under `checkJs: true`. A JS file
/// with `checkJs` unset is still bound, but the compiler keeps only a
/// fixed set of plain-JS errors there and `TS2307` is not one of them.
///
/// Only line comments before the first token count as directives — the
/// compiler reads them from the file's leading comments, so a
/// `/* @ts-check */` block or a `// @ts-check` below the first statement
/// is ignored.
fn script_is_type_checked(
    program: &Program<'_>,
    source_type: SourceType,
    check_js: Option<bool>,
) -> bool {
    let first_token = program
        .directives
        .first()
        .map(|d| d.span.start)
        .into_iter()
        .chain(program.body.first().map(|s| s.span().start))
        .min()
        .unwrap_or(program.span.end);
    let directive = program
        .comments
        .iter()
        .filter(|c| c.span.end <= first_token && c.is_line())
        .filter_map(|c| check_directive(c.content_span().source_text(program.source_text)))
        .next_back();
    match directive {
        Some(enabled) => enabled,
        None => source_type.is_typescript() || check_js == Some(true),
    }
}

/// `Some(true)` for a `@ts-check` line-comment pragma, `Some(false)` for
/// `@ts-nocheck`, `None` otherwise. `text` is the comment body after
/// `//`; the compiler also accepts a third slash and matches the pragma
/// name case-insensitively, up to whitespace or `:`.
fn check_directive(text: &str) -> Option<bool> {
    let text = text.strip_prefix('/').unwrap_or(text).trim_start();
    let name = text
        .strip_prefix('@')?
        .split(|c: char| c.is_whitespace() || c == ':')
        .next()?;
    if name.eq_ignore_ascii_case("ts-check") {
        Some(true)
    } else if name.eq_ignore_ascii_case("ts-nocheck") {
        Some(false)
    } else {
        None
    }
}

/// A declaration file by TypeScript's naming rule: `.d.ts`, `.d.mts`,
/// `.d.cts`, or an arbitrary-extension declaration such as
/// `Foo.d.svelte.ts` (a `.d.` anywhere in a `.ts` file name).
fn is_declaration_file(file: &Path) -> bool {
    let Some(name) = file.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    name.ends_with(".d.mts")
        || name.ends_with(".d.cts")
        || (name.ends_with(".ts") && name.contains(".d."))
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

/// Does any `.d.ts` under `dir` declare the wildcard? Directories are
/// read in parallel: the walk covers the whole workspace, which can hold
/// a native-app tree (100k+ files) beside `src/`.
fn dir_declares_svelte_wildcard(dir: &Path) -> bool {
    let skipped = dir
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(is_skipped_scan_dir);
    !skipped && subtree_declares_svelte_wildcard(dir)
}

fn is_skipped_scan_dir(name: &str) -> bool {
    matches!(
        name,
        "node_modules" | ".svelte-kit" | ".git" | ".cache" | ".svelte-check"
    )
}

fn subtree_declares_svelte_wildcard(dir: &Path) -> bool {
    use rayon::prelude::*;

    let Ok(read) = std::fs::read_dir(dir) else {
        return false;
    };
    let mut subdirs = Vec::new();
    for entry in read.filter_map(Result::ok) {
        // Not followed: a symlinked directory is not descended and a
        // symlinked `.d.ts` is not read.
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        let name = entry.file_name();
        let name = name.to_str();
        if file_type.is_dir() {
            if !name.is_some_and(is_skipped_scan_dir) {
                subdirs.push(entry.path());
            }
        } else if file_type.is_file()
            && name.is_some_and(|n| n.ends_with(".d.ts"))
            && std::fs::read_to_string(entry.path())
                .ok()
                .is_some_and(|src| declares_svelte_wildcard(&src))
        {
            return true;
        }
    }
    subdirs
        .par_iter()
        .any(|d| subtree_declares_svelte_wildcard(d))
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
        let (fragment, _) = svn_parser::parse_all_template_runs(src, &doc.template.text_runs);
        svelte_refs::collect_component(&doc, &fragment, src, svn_parser::is_ts_svelte(src))
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

    fn script_checked(name: &str, src: &str, check_js: Option<bool>) -> bool {
        let allocator = Allocator::default();
        let source_type = SourceType::from_path(name).unwrap();
        let parsed = Parser::new(&allocator, src, source_type).parse();
        script_is_type_checked(&parsed.program, source_type, check_js)
    }

    #[test]
    fn ts_files_are_checked_unless_nocheck() {
        assert!(script_checked("a.ts", "import x from './x.svelte'", None));
        assert!(script_checked("a.mts", "export {}", Some(false)));
        assert!(!script_checked("a.ts", "// @ts-nocheck\nexport {}", None));
    }

    #[test]
    fn js_files_follow_check_js_and_directives() {
        assert!(!script_checked("a.js", "export {}", None));
        assert!(!script_checked("a.js", "export {}", Some(false)));
        assert!(script_checked("a.js", "export {}", Some(true)));
        assert!(script_checked(
            "a.js",
            "// @ts-check\nexport {}",
            Some(false)
        ));
        assert!(script_checked("a.js", "/// @TS-Check: on\nexport {}", None));
        assert!(!script_checked(
            "a.js",
            "// @ts-nocheck\nexport {}",
            Some(true)
        ));
    }

    #[test]
    fn last_leading_line_directive_wins() {
        assert!(!script_checked(
            "a.js",
            "// @ts-check\n// @ts-nocheck\nexport {}",
            None
        ));
        assert!(script_checked(
            "a.js",
            "// @ts-nocheck\n\n// @ts-check\nexport {}",
            None
        ));
        // Not leading, or not a line comment: ignored.
        assert!(!script_checked(
            "a.js",
            "const x = 1;\n// @ts-check\nexport {}",
            None
        ));
        assert!(!script_checked("a.js", "/* @ts-check */\nexport {}", None));
        assert!(!script_checked("a.js", "// @ts-checked\nexport {}", None));
        // A directive prologue is the first token.
        assert!(!script_checked(
            "a.js",
            "'use strict';\n// @ts-check\nexport {}",
            None
        ));
    }

    #[test]
    fn declaration_file_names() {
        assert!(is_declaration_file(Path::new("src/app.d.ts")));
        assert!(is_declaration_file(Path::new("x.d.mts")));
        assert!(is_declaration_file(Path::new("Foo.d.svelte.ts")));
        assert!(!is_declaration_file(Path::new("state.svelte.ts")));
        assert!(!is_declaration_file(Path::new("d.ts.js")));
    }

    /// End to end for a plain module: only the missing specifier fires,
    /// at the opening quote, and a file the compiler does not check
    /// reports nothing.
    #[test]
    fn script_reports_missing_specifier_only() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("src/Here.svelte"), "").unwrap();
        let tsconfig = root.join("tsconfig.json");
        std::fs::write(
            &tsconfig,
            r#"{ "compilerOptions": { "skipLibCheck": true } }"#,
        )
        .unwrap();
        let resolver = SvelteImportResolver::new(root, &tsconfig);

        let main = root.join("src/main.ts");
        let src = "import Here from './Here.svelte';\nimport Gone from './Gone.svelte';\n";
        let diags = missing_svelte_import_diagnostics_in_script(&main, src, &resolver).diagnostics;
        assert_eq!(diags.len(), 1);
        assert_eq!((diags[0].line, diags[0].column), (2, 18));
        assert_eq!(
            diags[0].message,
            "Cannot find module './Gone.svelte' or its corresponding type declarations."
        );

        let none = |file: &Path, resolver: &SvelteImportResolver| {
            let found = missing_svelte_import_diagnostics_in_script(file, src, resolver);
            found.diagnostics.is_empty() && found.stub_targets.is_empty()
        };
        assert!(none(&root.join("src/plain.js"), &resolver));
        assert!(none(&root.join("src/types.d.ts"), &resolver));
        assert!(none(&main, &SvelteImportResolver::disabled()));
    }

    #[test]
    fn relative_specifiers_follow_typescript() {
        for relative in [".", "..", "./X.svelte", "../X.svelte", "/abs/X.svelte"] {
            assert!(is_relative_specifier(relative), "{relative}");
        }
        for mapped in [
            "$lib/X.svelte",
            "pkg/X.svelte",
            ".hidden/X.svelte",
            "~/X.svelte",
        ] {
            assert!(!is_relative_specifier(mapped), "{mapped}");
        }
    }

    #[test]
    fn one_script_stub_per_target() {
        let file = |p: &str| StubTarget::SvelteFile(PathBuf::from(p));
        let alias = || StubTarget::Specifier("$lib/G.svelte".into());
        let stubs = module_stubs([file("/w/src/B.svelte"), alias(), file("/w/src/B.svelte")]);
        assert_eq!(
            stubs,
            vec![
                (file("/w/src/B.svelte"), MISSING_MODULE_STUB),
                (alias(), MISSING_MODULE_STUB),
            ]
        );
        // A script: nothing that would make the compiler load it as a
        // module.
        assert!(!MISSING_MODULE_STUB.contains("import"));
        assert!(!MISSING_MODULE_STUB.contains("export"));
    }

    /// A relative target is named by its absolute path, an aliased one by
    /// the specifier as written.
    #[test]
    fn missing_imports_carry_their_stub_target() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("src")).unwrap();
        let tsconfig = root.join("tsconfig.json");
        std::fs::write(&tsconfig, "{}").unwrap();
        let resolver = SvelteImportResolver::new(root, &tsconfig);
        let src = "import A from '../src/./Gone.svelte';\nimport { B } from '$lib/Gone.svelte';\n";
        let found =
            missing_svelte_import_diagnostics_in_script(&root.join("src/main.ts"), src, &resolver);
        assert_eq!(
            found.stub_targets,
            vec![
                StubTarget::SvelteFile(root.join("src/Gone.svelte")),
                StubTarget::Specifier("$lib/Gone.svelte".into()),
            ]
        );
    }

    /// svelte-check's directive and the component's language decide,
    /// before `checkJs`.
    #[test]
    fn component_checking_follows_svelte_check() {
        let checked = |src: &str, check_js: Option<bool>| {
            let (doc, _) = svn_parser::parse_sections(src);
            component_is_type_checked(&doc, svn_parser::is_ts_svelte(src), check_js)
        };
        let js = "<script>\nlet x;\n</script>";
        let ts = "<script lang=\"ts\">\nlet x;\n</script>";
        assert!(!checked(js, None));
        assert!(!checked(js, Some(false)));
        assert!(checked(js, Some(true)));
        assert!(checked(ts, Some(false)));
        assert!(checked("<script>\n// @ts-check\nlet x;\n</script>", None));
        assert!(!checked(
            "<script lang=\"ts\">// @ts-nocheck\n</script>",
            None
        ));
        // The first directive wins, unlike in a plain module.
        assert!(checked(
            "<script>// @ts-check\n// @ts-nocheck\n</script>",
            None
        ));
        // The instance script decides over the module script.
        assert!(!checked(
            "<script module>// @ts-check\n</script><script>let x;</script>",
            None
        ));
        // A component with no script at all is the language default.
        assert!(!checked("<p>hi</p>", None));
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
