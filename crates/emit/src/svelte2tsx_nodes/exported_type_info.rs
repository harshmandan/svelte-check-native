//! Export-declaration type-info capture.
//!
//! Mirrors the type-source extraction half of upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts`
//! (the part that builds each `ExportedName`'s type signature from
//! the declaration's annotation).
//!
//! Called once per `export const|let|function|class` declaration by
//! [`crate::process_instance_script_content`]; the resulting
//! [`ExportedLocalInfo`] vector feeds `build_exports_object` in
//! [`crate::props_emit`].

use oxc_ast::ast::{BindingPattern, Declaration};
use oxc_span::GetSpan;
use smol_str::SmolStr;

use crate::process_instance_script_content::ExportedLocalInfo;

/// Mirrors upstream svelte2tsx's
/// `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts:79+`
/// (`addExportedName` -> `addPossibleExport`). For each export shape
/// we know how to surface as a typed slot in the rendered
/// component's `Exports` intersection, push an `ExportedLocalInfo`.
///
/// Function / let / const with explicit annotations carry their
/// annotation source text verbatim so consumers see the user's
/// declared shape; un-annotated declarations fall back to `None`
/// (caller emits `typeof <name>` / `any`).
///
/// Class exports also fall back to `None`; surfacing an instance
/// type from a class export needs `InstanceType<typeof ClassName>`,
/// which requires a module-scope reference we don't have (the class
/// body is body-scoped after the `export` prefix is stripped).
pub(crate) fn collect_export_type_infos(
    decl: &Declaration<'_>,
    content: &str,
    out: &mut Vec<ExportedLocalInfo>,
) {
    match decl {
        Declaration::FunctionDeclaration(f) => {
            let Some(id) = &f.id else { return };
            let name = SmolStr::from(id.name.as_str());
            // Always `type_source = None` so build_exports_object emits
            // `typeof <name>` — the function decl is hoisted in $$render's
            // scope, so the reference resolves and TS reads the full
            // (declared or inferred) signature. Mirrors upstream
            // svelte2tsx's `handleExportFunctionOrClass` (adds the export
            // with NO `type`) → `createReturnElementsType` emits `typeof
            // ${key}`.
            //
            // We previously reconstructed a function-type literal
            // (`{params} => {ret}`) when the decl had a return annotation.
            // That is invalid TS the moment a parameter carries a default
            // (`(name = "world") => string` — parameter initializers are
            // illegal in a type literal) and diverges from upstream for no
            // benefit; `typeof <name>` already conveys the full signature.
            out.push(ExportedLocalInfo {
                name,
                type_source: None,
                is_let: false,
                has_init: true,
            });
        }
        Declaration::VariableDeclaration(v) => {
            let is_let = matches!(v.kind, oxc_ast::ast::VariableDeclarationKind::Let);
            for d in &v.declarations {
                // Only simple `name: T = ...` patterns — destructures
                // and anonymous-binding cases we surface as `any`.
                let BindingPattern::BindingIdentifier(id) = &d.id else {
                    continue;
                };
                let name = SmolStr::from(id.name.as_str());
                let type_source = d.type_annotation.as_deref().map(|ta| {
                    let span = GetSpan::span(&ta.type_annotation);
                    content[span.start as usize..span.end as usize].to_string()
                });
                let has_init = d.init.is_some();
                out.push(ExportedLocalInfo {
                    name,
                    type_source,
                    is_let,
                    has_init,
                });
            }
        }
        // `export class Foo {}` — surface as `any`. Classes exported
        // from a component are rare and their instance shape requires
        // body-scope reference we don't have at module scope.
        Declaration::ClassDeclaration(c) => {
            if let Some(id) = &c.id {
                out.push(ExportedLocalInfo {
                    name: SmolStr::from(id.name.as_str()),
                    type_source: None,
                    is_let: false,
                    has_init: true,
                });
            }
        }
        _ => {}
    }
}
