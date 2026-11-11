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
use svn_core::{PositionMap, Range};
use svn_parser::ScriptLang;

use crate::missing_svelte_imports::{
    EnhancementDiagnostic, component_is_type_checked, resolve_options, script_is_type_checked,
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
    /// Every name it exports; `None` when that can't be told (`export =`,
    /// or an `export *` whose module can't be read).
    exports: Option<HashSet<String>>,
}

/// The diagnostics at every import, in a file of `program_files`, of one
/// of the `rejected` components (path and source).
pub fn rejected_import_diagnostics(
    tsconfig: &Path,
    program_files: &[PathBuf],
    rejected: &[(PathBuf, String)],
) -> Vec<EnhancementDiagnostic> {
    if rejected.is_empty() {
        return Vec::new();
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
    let resolver = Resolver::new(resolve_options(tsconfig));
    let fallbacks: HashMap<PathBuf, Fallback> = rejected
        .iter()
        .map(|(path, source)| {
            let path = canonical(path);
            let fallback = fallback(&path, source, &resolver);
            (path, fallback)
        })
        .collect();

    let mut out = Vec::new();
    let mut seen: HashSet<&Path> = HashSet::new();
    for file in program_files {
        if !seen.insert(file) || file.components().any(|c| c.as_os_str() == "node_modules") {
            continue;
        }
        let Ok(source) = std::fs::read_to_string(file) else {
            continue;
        };
        if !source.contains(".svelte") {
            continue;
        }
        let Some(refs) = checked_refs(file, &source, options.check_js) else {
            continue;
        };
        let dir = file.parent().unwrap_or_else(|| Path::new("."));
        let pm = PositionMap::new(&source);
        let mut push = |start: u32, end: u32, code: u32, message: String| {
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
            let path = fallback.path.display();
            if !(fallback.es_syntax || force_modules) {
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
                    push(
                        name.start,
                        name.end,
                        2305,
                        format!(
                            "Module '\"{}\"' has no exported member '{}'.",
                            r.specifier, name.name
                        ),
                    );
                }
            }
        }
    }
    out
}

fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
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
            exports: Some(HashSet::new()),
        };
    };
    let ts = script.lang == ScriptLang::Ts;
    let allocator = Allocator::default();
    let parsed = svn_parser::parse_script_body(&allocator, script.content, script.lang);
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    let mut visited = HashSet::from([path.to_path_buf()]);
    Fallback {
        path: path.to_path_buf(),
        es_syntax: has_module_syntax(&parsed.program),
        ts,
        exports: exports(&parsed.program, dir, resolver, &mut visited),
    }
}

/// TypeScript's external-module indicator: an `import`, `export` or
/// `import x = require()`.
fn has_module_syntax(program: &Program<'_>) -> bool {
    program.body.iter().any(|statement| {
        statement.is_module_declaration()
            || matches!(
                statement,
                Statement::TSImportEqualsDeclaration(d) if d.module_reference.is_external()
            )
    })
}

/// The names `program` exports, following `export *` into the modules
/// it names (which never re-export a `default`).
fn exports(
    program: &Program<'_>,
    dir: &Path,
    resolver: &Resolver,
    visited: &mut HashSet<PathBuf>,
) -> Option<HashSet<String>> {
    let mut names = HashSet::new();
    for statement in &program.body {
        let Some(declaration) = statement.as_module_declaration() else {
            continue;
        };
        match declaration {
            ModuleDeclaration::ExportDeclaration(export) => match &export.declaration {
                Declaration::VariableDeclaration(v) => {
                    for declarator in &v.declarations {
                        for id in declarator.id.get_binding_identifiers() {
                            names.insert(id.name.to_string());
                        }
                    }
                }
                other => {
                    if let Some(id) = other.id() {
                        names.insert(id.name.to_string());
                    }
                }
            },
            ModuleDeclaration::ExportNamedDeclaration(named) => {
                for specifier in &named.specifiers {
                    names.insert(specifier.exported.name().to_string());
                }
            }
            ModuleDeclaration::ExportFromDeclaration(from) => {
                for specifier in &from.specifiers {
                    names.insert(specifier.exported.name().to_string());
                }
            }
            ModuleDeclaration::ExportDefaultDeclaration(_) => {
                names.insert("default".to_string());
            }
            ModuleDeclaration::ExportAllDeclaration(all) => match &all.exported {
                Some(name) => {
                    names.insert(name.name().to_string());
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
                    if parsed.fatal_error {
                        return None;
                    }
                    let target_dir = target.parent().unwrap_or_else(|| Path::new("."));
                    let mut inner = exports(&parsed.program, target_dir, resolver, visited)?;
                    inner.remove("default");
                    names.extend(inner);
                }
            },
            ModuleDeclaration::TSExportAssignment(_) => return None,
            ModuleDeclaration::ImportDeclaration(_)
            | ModuleDeclaration::TSNamespaceExportDeclaration(_) => {}
        }
    }
    Some(names)
}
