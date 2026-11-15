//! Imports of a component svelte2tsx can't convert.
//!
//! `svelte-check --tsgo` writes no code for such a component, so an import
//! of it resolves through svelte's `*.svelte` wildcard and is never
//! questioned. The default engine's language server falls back to the
//! component's script instead (the instance script, else the module
//! script, else nothing): that text is what the component's importers
//! see. TypeScript then reports at each import of it:
//!
//! - `TS2306` at the specifier when the text has no `import` / `export`
//!   and so is no module;
//! - `TS1192` at a default import when it is a module without a default
//!   export (unless TypeScript allows a synthetic one);
//! - `TS2305` at a name in braces that it does not export.
//!
//! Only in a file the compiler type-checks, as for `TS2307`.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use oxc_allocator::Allocator;
use oxc_ast::ast::{Declaration, ModuleDeclaration, Program, Statement};
use oxc_parser::Parser;
use oxc_resolver::Resolver;
use oxc_span::SourceType;
use rayon::prelude::*;
use svn_core::{PositionMap, Range};
use svn_parser::ScriptLang;

use crate::missing_svelte_imports::{
    EnhancementDiagnostic, SvelteImportResolver, canonical, component_is_type_checked,
    resolve_options, script_is_type_checked,
};
use crate::svelte_refs::{self, SvelteImportRef};

/// What a rejected component's importers see of it.
struct Fallback {
    /// The component's path as TypeScript names it in messages.
    path: PathBuf,
    /// The text has `import` / `export` syntax.
    es_syntax: bool,
    /// TypeScript reads it as TypeScript (a `lang="ts"` script).
    ts: bool,
    /// Every name it exports, in the order it exports them; `None` when
    /// that can't be told (`export =`, or an `export *` whose module
    /// can't be read).
    exports: Option<Vec<String>>,
    /// The names its script declares or imports at the top level.
    locals: HashSet<String>,
    /// Local names exported under another name (`export { a as b }`).
    exported_as: HashMap<String, String>,
    /// What the default export is called in the module, where it has
    /// a name there (`export default local`, `export default function
    /// f`): the name TypeScript shows it by.
    default_name: Option<String>,
}

/// What the imports of rejected components come to.
#[derive(Default)]
pub struct RejectedImports {
    /// The diagnostics TypeScript gives at those imports.
    pub diagnostics: Vec<EnhancementDiagnostic>,
    /// The `// @ts-expect-error` comments (file, 1-based line) that one
    /// of those diagnostics satisfies. The compiler that checked the
    /// program saw no error there and calls each one unused (`TS2578`);
    /// with the error it expects, it is not.
    pub satisfied_expectations: Vec<(PathBuf, u32)>,
    /// Where a name is imported from a rejected component (file,
    /// 1-based line and column). The compiler that checked the program
    /// resolved the import to svelte's `*.svelte` declaration and judged
    /// the name against that; what it says there is about the wrong
    /// module, and `diagnostics` holds what holds of the right one.
    /// Only its verdicts on what the module exports are about the
    /// module (see [`RejectedImports::judged_wrongly`]).
    pub judged_names: Vec<(PathBuf, u32, u32)>,
}

/// The codes TypeScript gives at an imported name for what the module it
/// names does or does not export: no default export, no such member (with
/// or without a suggestion), and a member the module only declares.
const EXPORT_VERDICTS: [u32; 6] = [1192, 2305, 2459, 2460, 2614, 2724];

impl RejectedImports {
    /// Whether a diagnostic with `code` at `file`, `line`, `column`
    /// (1-based) is the compiler's verdict on a name imported from a
    /// rejected component, judged against svelte's `*.svelte`
    /// declaration. What it says there that does not depend on the
    /// module, such as an unused import, stands.
    pub fn judged_wrongly(&self, file: &Path, line: u32, column: u32, code: u32) -> bool {
        EXPORT_VERDICTS.contains(&code)
            && self
                .judged_names
                .iter()
                .any(|(f, l, c)| f == file && *l == line && *c == column)
    }
}

/// The comment directives in `file`'s source as TypeScript's scanner
/// finds them: the 0-based line of each `@ts-ignore` / `@ts-expect-error`
/// comment, and whether it expects an error. A component's are those of
/// its scripts. A line comment is one when it starts with two or three
/// slashes and whitespace; a block comment when its last line starts,
/// after whitespace, with any `/` and `*` and whitespace.
fn comment_directives(file: &Path, source: &str) -> HashMap<u32, bool> {
    use oxc_ast::CommentKind;
    let mut out = HashMap::new();
    let pm = PositionMap::new(source);
    let mut read = |program: &Program<'_>, base: u32| {
        for comment in &program.comments {
            let start = base + comment.span.start;
            let end = base + comment.span.end;
            let Some(text) = source.get(start as usize..end as usize) else {
                continue;
            };
            let (at, directive) = match comment.kind {
                CommentKind::Line => {
                    let after = text.strip_prefix("//").unwrap_or(text);
                    let after = after.strip_prefix('/').unwrap_or(after);
                    (
                        start,
                        after.trim_start_matches(svn_parser::is_svelte_whitespace),
                    )
                }
                CommentKind::SingleLineBlock | CommentKind::MultiLineBlock => {
                    // The scanner reads only the comment's last line.
                    let last_line = text
                        .rfind(['\n', '\r', '\u{2028}', '\u{2029}'])
                        .map_or(0, |i| {
                            i + text[i..].chars().next().map_or(1, char::len_utf8)
                        });
                    let line = &text[last_line..];
                    let after = line
                        .trim_start_matches(svn_parser::is_svelte_whitespace)
                        .trim_start_matches(['/', '*'])
                        .trim_start_matches(svn_parser::is_svelte_whitespace);
                    (start + last_line as u32, after)
                }
                _ => continue,
            };
            let expects = if directive.starts_with("@ts-expect-error") {
                true
            } else if directive.starts_with("@ts-ignore") {
                false
            } else {
                continue;
            };
            out.insert(pm.position_of(at).line, expects);
        }
    };
    if file.extension().is_some_and(|e| e == "svelte") {
        let (doc, _) = svn_parser::parse_sections(source);
        for script in [doc.instance_script.as_ref(), doc.module_script.as_ref()]
            .into_iter()
            .flatten()
        {
            let allocator = Allocator::default();
            let parsed = svn_parser::parse_script_body(&allocator, script.content, script.lang);
            read(&parsed.program, script.content_range.start);
        }
    } else if let Ok(source_type) = SourceType::from_path(file) {
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, source, source_type).parse();
        read(&parsed.program, 0);
    }
    out
}

/// The comment directive that applies to the line `offset` is on, as
/// TypeScript looks for it: on the line before, or before that across
/// blank lines and lines that are only a `//` comment. Its 0-based
/// line, and whether it expects an error.
fn comment_directive(
    source: &str,
    directives: &HashMap<u32, bool>,
    offset: u32,
) -> Option<(u32, bool)> {
    let before = source.get(..offset as usize)?;
    // The lines before the one the diagnostic is on, nearest first.
    let head = &before[..before.rfind('\n').map_or(0, |i| i + 1)];
    let lines_before = head.bytes().filter(|&b| b == b'\n').count() as u32;
    for (index, line) in (0..lines_before).rev().zip(head.lines().rev()) {
        if let Some(&expects) = directives.get(&index) {
            return Some((index, expects));
        }
        let trimmed = line.trim();
        if !(trimmed.is_empty() || trimmed.starts_with("//")) {
            return None;
        }
    }
    None
}

/// The diagnostics at every import, in a file of `program_files`, of one
/// of the `rejected` components (path and source).
pub fn rejected_import_diagnostics(
    resolver: &SvelteImportResolver,
    tsconfig: &Path,
    program_files: &[PathBuf],
    rejected: &[(PathBuf, String)],
) -> RejectedImports {
    let Some(resolver) = resolver.resolver() else {
        return RejectedImports::default();
    };
    if rejected.is_empty() {
        return RejectedImports::default();
    }
    let options = svn_core::tsconfig::load(tsconfig)
        .map(|config| config.compiler_options)
        .unwrap_or_default();
    let force_modules = options
        .raw
        .get("moduleDetection")
        .and_then(|v| v.as_str())
        .is_some_and(|v| v.eq_ignore_ascii_case("force"));
    let synthetic_defaults = options.allow_synthetic_default_imports.unwrap_or(
        options.es_module_interop == Some(true)
            || options
                .module
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case("system"))
            || matches!(
                options.module_resolution,
                Some(svn_core::tsconfig::ModuleResolution::Bundler)
            ),
    );
    // What an `export *` in a fallback script names is read from the
    // module's types, as TypeScript reads it.
    let types_resolver = resolver.clone_with_options(type_resolve_options(tsconfig));
    let fallbacks: HashMap<PathBuf, Fallback> = rejected
        .par_iter()
        .map(|(path, source)| {
            let path = canonical(path);
            let fallback = fallback(&path, source, &types_resolver);
            (path, fallback)
        })
        .collect();

    let mut seen: HashSet<&Path> = HashSet::new();
    let files: Vec<&PathBuf> = program_files
        .iter()
        .filter(|file| {
            seen.insert(file) && !file.components().any(|c| c.as_os_str() == "node_modules")
        })
        .collect();
    let per_file: Vec<RejectedImports> = files
        .par_iter()
        .map(|file| {
            file_diagnostics(
                file,
                resolver,
                &fallbacks,
                &options,
                force_modules,
                synthetic_defaults,
            )
        })
        .collect();
    let mut all = RejectedImports::default();
    for found in per_file {
        all.diagnostics.extend(found.diagnostics);
        all.satisfied_expectations
            .extend(found.satisfied_expectations);
        all.judged_names.extend(found.judged_names);
    }
    all
}

/// [`rejected_import_diagnostics`] for one file of the program.
fn file_diagnostics(
    file: &Path,
    resolver: &Resolver,
    fallbacks: &HashMap<PathBuf, Fallback>,
    options: &svn_core::tsconfig::CompilerOptions,
    force_modules: bool,
    synthetic_defaults: bool,
) -> RejectedImports {
    let mut out = Vec::new();
    let mut satisfied_expectations = Vec::new();
    let mut judged_names = Vec::new();
    {
        let Ok(source) = std::fs::read_to_string(file) else {
            return RejectedImports::default();
        };
        if !source.contains(".svelte") {
            return RejectedImports::default();
        }
        // TypeScript reports nothing in a declaration file it was told
        // not to check.
        if options.skip_lib_check == Some(true)
            && crate::missing_svelte_imports::is_declaration_file(file)
        {
            return RejectedImports::default();
        }
        let Some(refs) = checked_refs(file, &source, options.check_js) else {
            return RejectedImports::default();
        };
        let file = file.to_path_buf();
        let dir = file.parent().unwrap_or_else(|| Path::new("."));
        let pm = PositionMap::new(&source);
        // Read only when an import here names a rejected component.
        let directives = std::cell::OnceCell::new();
        let mut push = |start: u32, end: u32, code: u32, message: String| {
            let directives = directives.get_or_init(|| comment_directives(&file, &source));
            // A comment directive above the line takes the error.
            match comment_directive(&source, directives, start) {
                Some((line, true)) => {
                    satisfied_expectations.push((file.clone(), line + 1));
                    return;
                }
                Some((_, false)) => return,
                None => {}
            }
            let (s, e) = pm.range_positions(Range::new(start, end));
            out.push(EnhancementDiagnostic {
                file: file.clone(),
                line: s.line.saturating_add(1),
                column: s.character.saturating_add(1),
                end_line: e.line.saturating_add(1),
                end_column: e.character.saturating_add(1),
                code,
                message,
            });
        };
        for r in refs {
            let Some(target) = resolver
                .resolve(dir, &r.specifier)
                .ok()
                .map(|resolution| canonical(resolution.path()))
            else {
                continue;
            };
            let Some(fallback) = fallbacks.get(&target) else {
                continue;
            };
            for name in &r.names {
                let at = pm.position_of(name.start);
                judged_names.push((
                    file.clone(),
                    at.line.saturating_add(1),
                    at.character.saturating_add(1),
                ));
            }
            // TypeScript and tsgo name a file with forward slashes.
            let path = fallback.path.to_string_lossy().replace('\\', "/");
            if !(fallback.es_syntax || force_modules) {
                // A module imported for its side effects need not be
                // one.
                if r.side_effect {
                    continue;
                }
                push(
                    r.start,
                    r.end,
                    2306,
                    format!("File '{path}' is not a module."),
                );
                continue;
            }
            for name in &r.names {
                let exported = fallback.exports.as_ref().map(|e| e.contains(&name.name));
                if name.default_import {
                    // TypeScript files never get a synthetic default; a
                    // JavaScript file does when it has no module syntax.
                    let synthetic = synthetic_defaults && !fallback.ts && !fallback.es_syntax;
                    if exported == Some(false) && !synthetic {
                        push(
                            name.start,
                            name.end,
                            1192,
                            format!("Module '\"{path}\"' has no default export."),
                        );
                    }
                } else if exported == Some(false) {
                    // What TypeScript says of a missing member, in the
                    // order it looks: a near spelling among the exports,
                    // a default export to import instead, a declaration
                    // the module keeps to itself.
                    let module = &r.specifier;
                    let missing = &name.name;
                    let candidates = fallback.exports.iter().flatten().map(String::as_str);
                    let has_default = fallback
                        .exports
                        .as_ref()
                        .is_some_and(|exports| exports.iter().any(|e| e == "default"));
                    // The default export is matched as `default` and
                    // shown by the name it has in the module.
                    let suggestion = spelling_suggestion(missing, candidates).map(|found| {
                        match (&fallback.default_name, found) {
                            (Some(name), "default") => name.as_str(),
                            _ => found,
                        }
                    });
                    let (code, message) = if let Some(suggestion) = suggestion {
                        (
                            2724,
                            format!(
                                "'\"{module}\"' has no exported member named '{missing}'. Did you mean '{suggestion}'?"
                            ),
                        )
                    } else if has_default {
                        (
                            2614,
                            format!(
                                "Module '\"{module}\"' has no exported member '{missing}'. Did you mean to use 'import {missing} from \"{module}\"' instead?"
                            ),
                        )
                    } else if let Some(other) = fallback.exported_as.get(missing) {
                        (
                            2460,
                            format!(
                                "Module '\"{module}\"' declares '{missing}' locally, but it is exported as '{other}'."
                            ),
                        )
                    } else if fallback.locals.contains(missing) {
                        (
                            2459,
                            format!(
                                "Module '\"{module}\"' declares '{missing}' locally, but it is not exported."
                            ),
                        )
                    } else {
                        (
                            2305,
                            format!("Module '\"{module}\"' has no exported member '{missing}'."),
                        )
                    };
                    push(name.start, name.end, code, message);
                }
            }
        }
    }
    RejectedImports {
        diagnostics: out,
        satisfied_expectations,
        judged_names,
    }
}

/// [`resolve_options`] reading a package's types, as TypeScript does:
/// its `types` / `typings` field before `main`, and a `.js` specifier
/// reaching the `.ts` or `.d.ts` beside it.
fn type_resolve_options(tsconfig: &Path) -> oxc_resolver::ResolveOptions {
    let alias = |js: &str, ts: &[&str]| {
        (
            js.to_string(),
            ts.iter().map(|e| (*e).to_string()).collect::<Vec<_>>(),
        )
    };
    oxc_resolver::ResolveOptions {
        main_fields: ["types", "typings", "main"]
            .iter()
            .map(|f| (*f).to_string())
            .collect(),
        extension_alias: vec![
            alias(".js", &[".ts", ".tsx", ".d.ts", ".js"]),
            alias(".mjs", &[".mts", ".d.mts", ".mjs"]),
            alias(".cjs", &[".cts", ".d.cts", ".cjs"]),
        ],
        ..resolve_options(tsconfig)
    }
}

/// The `.svelte` imports of a file the compiler type-checks.
fn checked_refs(file: &Path, source: &str, check_js: Option<bool>) -> Option<Vec<SvelteImportRef>> {
    if file.extension().is_some_and(|e| e == "svelte") {
        let (doc, _) = svn_parser::parse_sections(source);
        let is_ts = svn_parser::is_ts_svelte(source);
        if !component_is_type_checked(&doc, is_ts, check_js) {
            return None;
        }
        let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
        return Some(svelte_refs::collect_component(
            &doc, &fragment, source, is_ts,
        ));
    }
    let source_type = SourceType::from_path(file).ok()?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source, source_type).parse();
    if parsed.fatal_error || !script_is_type_checked(&parsed.program, source_type, check_js) {
        return None;
    }
    Some(svelte_refs::collect_module(
        &parsed.program,
        source_type.is_javascript(),
    ))
}

/// The language server's fallback for a component it can't convert.
fn fallback(path: &Path, source: &str, resolver: &Resolver) -> Fallback {
    let (doc, _) = svn_parser::parse_sections(source);
    let Some(script) = doc.instance_script.as_ref().or(doc.module_script.as_ref()) else {
        return Fallback {
            path: path.to_path_buf(),
            es_syntax: false,
            ts: false,
            exports: Some(Vec::new()),
            locals: HashSet::new(),
            exported_as: HashMap::new(),
            default_name: None,
        };
    };
    // The fallback is TypeScript when either script is.
    let ts = [doc.instance_script.as_ref(), doc.module_script.as_ref()]
        .into_iter()
        .flatten()
        .any(|script| script.lang == ScriptLang::Ts);
    let allocator = Allocator::default();
    let parsed = svn_parser::parse_script_body(&allocator, script.content, script.lang);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut visited = HashSet::from([path.to_path_buf()]);
    let (locals, exported_as, default_name) = top_level_names(&parsed.program);
    Fallback {
        path: path.to_path_buf(),
        es_syntax: has_module_syntax(&parsed.program),
        ts,
        exports: exports(&parsed.program, dir, resolver, &mut visited),
        locals,
        exported_as,
        default_name,
    }
}

/// TypeScript's external-module indicator: an `import`, `export` or
/// `import x = require()` statement, or `import.meta` anywhere.
fn has_module_syntax(program: &Program<'_>) -> bool {
    use oxc_ast_visit::Visit;
    struct ImportMeta(bool);
    impl<'a> Visit<'a> for ImportMeta {
        fn visit_import_meta(&mut self, _it: &oxc_ast::ast::ImportMeta) {
            self.0 = true;
        }
    }
    program.body.iter().any(|statement| {
        statement.is_module_declaration()
            || matches!(
                statement,
                Statement::TSImportEqualsDeclaration(d) if d.module_reference.is_external()
            )
    }) || {
        let mut finder = ImportMeta(false);
        finder.visit_program(program);
        finder.0
    }
}

/// Adds `name` unless it is there already.
fn add(names: &mut Vec<String>, name: &str) {
    if !names.iter().any(|n| n == name) {
        names.push(name.to_string());
    }
}

/// The names `program` declares or imports at its top level, the local
/// names it exports under another name, and the name of its default
/// export where that has one of its own.
fn top_level_names(
    program: &Program<'_>,
) -> (HashSet<String>, HashMap<String, String>, Option<String>) {
    use oxc_ast::ast::{
        ExportDefaultDeclarationKind, ImportDeclarationSpecifier, ModuleExportName,
    };
    let mut locals = HashSet::new();
    let mut exported_as = HashMap::new();
    let mut default_name = None;
    let declare = |declaration: &Declaration<'_>, locals: &mut HashSet<String>| match declaration {
        Declaration::VariableDeclaration(v) => {
            for declarator in &v.declarations {
                for id in declarator.id.get_binding_identifiers() {
                    locals.insert(id.name.to_string());
                }
            }
        }
        other => {
            if let Some(id) = other.id() {
                locals.insert(id.name.to_string());
            }
        }
    };
    for statement in &program.body {
        if let Some(declaration) = statement.as_declaration() {
            declare(declaration, &mut locals);
            continue;
        }
        match statement.as_module_declaration() {
            Some(ModuleDeclaration::ImportDeclaration(import)) => {
                for specifier in import.specifiers.iter().flatten() {
                    let local = match specifier {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => &s.local,
                        ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                        ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
                    };
                    locals.insert(local.name.to_string());
                }
            }
            Some(ModuleDeclaration::ExportDeclaration(export)) => {
                declare(&export.declaration, &mut locals);
            }
            Some(ModuleDeclaration::ExportNamedDeclaration(named)) => {
                for specifier in &named.specifiers {
                    if let ModuleExportName::IdentifierReference(local) = &specifier.local
                        && specifier.exported.name() != local.name
                    {
                        exported_as
                            .entry(local.name.to_string())
                            .or_insert_with(|| specifier.exported.name().to_string());
                    }
                }
            }
            Some(ModuleDeclaration::ExportDefaultDeclaration(default)) => {
                default_name = match &default.declaration {
                    ExportDefaultDeclarationKind::Identifier(id) => Some(id.name.to_string()),
                    ExportDefaultDeclarationKind::FunctionDeclaration(f) => {
                        f.id.as_ref().map(|id| id.name.to_string())
                    }
                    ExportDefaultDeclarationKind::ClassDeclaration(c) => {
                        c.id.as_ref().map(|id| id.name.to_string())
                    }
                    _ => None,
                };
                if let Some(name) = &default_name
                    && !matches!(
                        &default.declaration,
                        ExportDefaultDeclarationKind::Identifier(_)
                    )
                {
                    locals.insert(name.clone());
                }
            }
            _ => {}
        }
    }
    (locals, exported_as, default_name)
}

/// TypeScript's spelling suggestion for `name` among `candidates`
/// (`getSpellingSuggestion`): the first candidate close enough, and
/// after it only one that is closer.
fn spelling_suggestion<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    let name_units: Vec<u16> = name.encode_utf16().collect();
    let length = name_units.len() as f64;
    let maximum_length_difference = (length * 0.34).floor().max(2.0);
    let mut best_distance = (length * 0.4).floor() + 1.0;
    let mut best = None;
    for candidate in candidates {
        let candidate_units: Vec<u16> = candidate.encode_utf16().collect();
        if candidate == name
            || (candidate_units.len() as f64 - length).abs() > maximum_length_difference
        {
            continue;
        }
        if candidate_units.len() < 3 && candidate.to_lowercase() != name.to_lowercase() {
            continue;
        }
        let Some(distance) = levenshtein_with_max(name, candidate, best_distance - 0.1) else {
            continue;
        };
        best_distance = distance;
        best = Some(candidate);
    }
    best
}

/// TypeScript's `levenshteinWithMax`: an insertion or a deletion costs
/// one, a change of letter two, a change of case a tenth. `None` past
/// `max`.
fn levenshtein_with_max(s1: &str, s2: &str, max: f64) -> Option<f64> {
    let a: Vec<char> = s1.chars().collect();
    let b: Vec<char> = s2.chars().collect();
    let big = max + 0.01;
    let mut previous: Vec<f64> = (0..=b.len()).map(|i| i as f64).collect();
    let mut current: Vec<f64> = vec![0.0; b.len() + 1];
    for i in 1..=a.len() {
        let c1 = a[i - 1];
        let i_f = i as f64;
        let min_j = (if i_f > max { i_f - max } else { 1.0 }).ceil() as usize;
        let max_j = (if b.len() as f64 > max + i_f {
            max + i_f
        } else {
            b.len() as f64
        })
        .floor() as usize;
        current[0] = i_f;
        let mut col_min = i_f;
        for cell in current.iter_mut().take(min_j.min(b.len() + 1)).skip(1) {
            *cell = big;
        }
        for j in min_j..=max_j.min(b.len()) {
            let same_letter = c1.to_lowercase().eq(b[j - 1].to_lowercase());
            let substitution = previous[j - 1] + if same_letter { 0.1 } else { 2.0 };
            let distance = if c1 == b[j - 1] {
                previous[j - 1]
            } else {
                (previous[j] + 1.0)
                    .min(current[j - 1] + 1.0)
                    .min(substitution)
            };
            current[j] = distance;
            col_min = col_min.min(distance);
        }
        for cell in current.iter_mut().skip(max_j + 1) {
            *cell = big;
        }
        if col_min > max {
            return None;
        }
        std::mem::swap(&mut previous, &mut current);
    }
    let result = previous[b.len()];
    (result <= max).then_some(result)
}

/// The names `program` exports, following `export *` into the modules
/// it names (which never re-export a `default`).
fn exports(
    program: &Program<'_>,
    dir: &Path,
    resolver: &Resolver,
    visited: &mut HashSet<PathBuf>,
) -> Option<Vec<String>> {
    let mut names: Vec<String> = Vec::new();
    for statement in &program.body {
        let Some(declaration) = statement.as_module_declaration() else {
            continue;
        };
        match declaration {
            ModuleDeclaration::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(v) => {
                    for declarator in &v.declarations {
                        for id in declarator.id.get_binding_identifiers() {
                            add(&mut names, id.name.as_str());
                        }
                    }
                }
                other => {
                    if let Some(id) = other.id() {
                        add(&mut names, id.name.as_str());
                    }
                }
            },
            ModuleDeclaration::ExportNamedDeclaration(named) => {
                for specifier in &named.specifiers {
                    add(&mut names, specifier.exported.name().as_str());
                }
            }
            ModuleDeclaration::ExportFromDeclaration(from) => {
                for specifier in &from.specifiers {
                    add(&mut names, specifier.exported.name().as_str());
                }
            }
            ModuleDeclaration::ExportDefaultDeclaration(_) => {
                add(&mut names, "default");
            }
            ModuleDeclaration::ExportAllDeclaration(all) => match &all.exported {
                Some(name) => {
                    add(&mut names, name.name().as_str());
                }
                None => {
                    let target = resolver.resolve(dir, all.source.value.as_str()?).ok()?;
                    let target = canonical(target.path());
                    if !visited.insert(target.clone()) {
                        continue;
                    }
                    let source = std::fs::read_to_string(&target).ok()?;
                    let source_type = SourceType::from_path(&target).ok()?;
                    let allocator = Allocator::default();
                    let parsed = Parser::new(&allocator, &source, source_type).parse();
                    // A script with no module syntax (CommonJS, say)
                    // exports what this can't read.
                    if parsed.fatal_error || !has_module_syntax(&parsed.program) {
                        return None;
                    }
                    let target_dir = target.parent().unwrap_or_else(|| Path::new("."));
                    let inner = exports(&parsed.program, target_dir, resolver, visited)?;
                    for name in inner.iter().filter(|name| *name != "default") {
                        add(&mut names, name);
                    }
                }
            },
            ModuleDeclaration::TSExportAssignment(_) => return None,
            ModuleDeclaration::ImportDeclaration(_)
            | ModuleDeclaration::TSNamespaceExportDeclaration(_) => {}
        }
    }
    Some(names)
}
