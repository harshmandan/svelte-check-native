//! `$store` auto-subscription declarations.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ImplicitStoreValues.ts`:
//! every variable declaration (or `$:` assignment, which the reactive
//! rewrite has already turned into a declaration) whose names are read
//! as `$name` gets `;let $name = __svn_store_get(name);` appended right
//! after it, wrapped in ignore comments; stores that come from imports
//! are declared once at the start of the render function.
//!
//! The declaration sits after the store's own, so `$name` carries the
//! store's value type without a forward reference, and a name that is
//! not a store fails the `__svn_store_get` call — inside the ignore
//! region, which leaves `$name` as `any` exactly as it does upstream.

use std::collections::HashSet;
use std::ops::Range;

use oxc_ast::ast::{
    Declaration, ImportDeclarationSpecifier, ImportOrExportKind, Program, Statement,
};
use smol_str::SmolStr;
use svn_parser::ScriptLang;

use crate::process_instance_script_content::collect_binding_pattern_names;
use crate::svelte4::compat::splice_insertions;

/// `;let $a = __svn_store_get(a);;let $b = __svn_store_get(b);` between
/// ignore comments — the exact text upstream appends.
pub(crate) fn store_declarations<'a>(names: impl IntoIterator<Item = &'a str>) -> String {
    let mut out = String::from("/*svn:ignore_start*/");
    for name in names {
        out.push_str(";let $");
        out.push_str(name);
        out.push_str(" = __svn_store_get(");
        out.push_str(name);
        out.push_str(");");
    }
    out.push_str("/*svn:ignore_end*/");
    out
}

/// Local names bound by value imports at the top level of `program`.
pub(crate) fn import_local_names(program: &Program<'_>, out: &mut HashSet<SmolStr>) {
    for stmt in &program.body {
        let Statement::ImportDeclaration(decl) = stmt else {
            continue;
        };
        if matches!(decl.import_kind, ImportOrExportKind::Type) {
            continue;
        }
        let Some(specifiers) = &decl.specifiers else {
            continue;
        };
        for spec in specifiers {
            let local = match spec {
                ImportDeclarationSpecifier::ImportSpecifier(s) => {
                    if matches!(s.import_kind, ImportOrExportKind::Type) {
                        continue;
                    }
                    &s.local
                }
                ImportDeclarationSpecifier::ImportDefaultSpecifier(s) => &s.local,
                ImportDeclarationSpecifier::ImportNamespaceSpecifier(s) => &s.local,
            };
            out.insert(SmolStr::from(local.name.as_str()));
        }
    }
}

/// Append the store declarations for `pending` after each top-level
/// variable declaration in the script spliced at `body` that declares
/// one of them. Handled names are removed from `pending`. Returns the
/// insertions as `(position, length)` pairs in pre-rewrite coordinates
/// for `EmitBuffer::adjust_token_map_for_insertions`.
pub(crate) fn attach_to_declarations(
    out: &mut String,
    body: &Range<usize>,
    pending: &mut Vec<SmolStr>,
) -> Vec<(u32, u32)> {
    if pending.is_empty() {
        return Vec::new();
    }
    let Some(src) = out.get(body.clone()) else {
        return Vec::new();
    };
    let alloc = oxc_allocator::Allocator::default();
    // TypeScript accepts every JS body, and a JS overlay's body can
    // still hold TS-only syntax a JS parse would give up on.
    let parsed = svn_parser::parse_script_body(&alloc, src, ScriptLang::Ts);
    if parsed.panicked {
        return Vec::new();
    }
    let mut insertions: Vec<(usize, String)> = Vec::new();
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(d) => d,
            Statement::ExportDeclaration(e) => match &e.declaration {
                Declaration::VariableDeclaration(d) => d,
                _ => continue,
            },
            _ => continue,
        };
        let mut names = Vec::new();
        for d in &decl.declarations {
            collect_binding_pattern_names(&d.id, &mut names);
        }
        let stores: Vec<&str> = names
            .iter()
            .map(SmolStr::as_str)
            .filter(|n| pending.iter().any(|p| p == n))
            .collect();
        if stores.is_empty() {
            continue;
        }
        // Upstream appends after the last declarator, before the `;`.
        let Some(last) = decl.declarations.last() else {
            continue;
        };
        let at = body.start + last.span.end as usize;
        let text = store_declarations(stores.iter().copied());
        pending.retain(|p| !stores.contains(&p.as_str()));
        insertions.push((at, text));
    }
    splice_insertions(out, &insertions)
}
