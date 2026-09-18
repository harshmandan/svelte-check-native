//! Which instance-script types move out of the render function.
//!
//! Port of upstream svelte2tsx's `HoistableInterfaces`
//! (`language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/HoistableInterfaces.ts`)
//! and of the `$$Generic` half of `processInstanceScriptContent`
//! (`nodesToMove`). The instance script becomes the body of the render
//! function, so its top-level `type` / `interface` declarations are
//! function-scoped unless moved. Upstream moves them in exactly two
//! cases:
//!
//! - a declaration named by a `$$Generic<Name>` constraint, which the
//!   render function's own type-parameter list references; and
//! - when the `$props()` rune carries a type, every declaration that
//!   can live at module scope — provided the props type itself can.
//!   With no typed `$props()` nothing moves, so a legacy component's
//!   `export type` stays in the function (where its `export` modifier
//!   is an error).
//!
//! A declaration can live at module scope when every type it names can
//! (and is not a script generic or shadowed by a module-script type)
//! and every value it reads through `typeof` is not an instance-script
//! declaration or a store.

use std::collections::{HashMap, HashSet};

use oxc_ast::ast::{
    Declaration, ImportDeclarationSpecifier, ImportOrExportKind, Program, Statement,
};
use smol_str::SmolStr;

use crate::svelte2tsx_nodes::type_deps::{TypeDeps, collect_type_node_deps, entity_name_root};

/// The type the `$props()` rune is annotated with (or given as its type
/// argument), as far as the hoisting decision cares.
#[derive(Debug, Clone, Default)]
pub enum PropsTypeShape {
    /// No typed `$props()`: nothing is hoisted.
    #[default]
    Untyped,
    /// A type reference; the root of its name.
    Reference(SmolStr),
    /// Any other type — upstream names it `$$ComponentProps`.
    Inline(TypeDepsHandle),
}

/// Opaque carrier for the dependencies of an inline props type.
#[derive(Debug, Clone, Default)]
pub struct TypeDepsHandle(pub(crate) TypeDeps);

impl PropsTypeShape {
    /// Classify the source text of a `$props()` type.
    pub fn from_type_text(text: &str) -> Self {
        let wrapped = format!("type __svn_props = {text};");
        let alloc = oxc_allocator::Allocator::default();
        let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
        let Some(Statement::TSTypeAliasDeclaration(alias)) = parsed.program.body.first() else {
            return Self::Untyped;
        };
        match &alias.type_annotation {
            oxc_ast::ast::TSType::TSTypeReference(r) => {
                Self::Reference(entity_name_root(&r.type_name))
            }
            ty => Self::Inline(TypeDepsHandle(collect_type_node_deps(ty))),
        }
    }
}

/// Everything outside the instance script's own statements that the
/// decision depends on.
#[derive(Debug, Clone, Default)]
pub struct HoistContext {
    /// Top-level statements of `<script module>`, if any.
    pub module_script: Option<String>,
    /// Names of the render function's type parameters.
    pub generic_names: Vec<SmolStr>,
    /// Constraint texts of `type X = $$Generic<Constraint>` declarations.
    pub generic_constraints: Vec<String>,
    /// Stores the component reads (`$name` → `name`).
    pub accessed_stores: Vec<SmolStr>,
    pub props_type: PropsTypeShape,
}

/// Names the module script declares in type space.
fn module_types(module: &str) -> HashSet<SmolStr> {
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, module, svn_parser::ScriptLang::Ts);
    let mut out = HashSet::new();
    for stmt in &parsed.program.body {
        match stmt {
            Statement::ImportDeclaration(import) => {
                let whole = import.import_kind == ImportOrExportKind::Type;
                for spec in import.specifiers.iter().flatten() {
                    let type_only = match spec {
                        ImportDeclarationSpecifier::ImportSpecifier(s) => {
                            whole || s.import_kind == ImportOrExportKind::Type
                        }
                        _ => whole,
                    };
                    if type_only {
                        out.insert(SmolStr::from(spec.local().name.as_str()));
                    }
                }
            }
            other => {
                if let Some(name) = type_space_name(other) {
                    out.insert(name);
                }
            }
        }
    }
    out
}

/// The name a type alias, interface, enum or namespace statement
/// declares, whether or not it is exported.
fn type_space_name(stmt: &Statement<'_>) -> Option<SmolStr> {
    fn decl_name<'s>(d: &'s Declaration<'_>) -> Option<&'s str> {
        match d {
            Declaration::TSTypeAliasDeclaration(t) => Some(t.id.name.as_str()),
            Declaration::TSInterfaceDeclaration(i) => Some(i.id.name.as_str()),
            Declaration::TSEnumDeclaration(e) => Some(e.id.name.as_str()),
            Declaration::TSNamespaceDeclaration(n) => Some(n.id.name.as_str()),
            Declaration::TSGlobalDeclaration(_) => Some("global"),
            _ => None,
        }
    }
    let name = match stmt {
        Statement::TSTypeAliasDeclaration(t) => Some(t.id.name.as_str()),
        Statement::TSInterfaceDeclaration(i) => Some(i.id.name.as_str()),
        Statement::TSEnumDeclaration(e) => Some(e.id.name.as_str()),
        Statement::TSNamespaceDeclaration(n) => Some(n.id.name.as_str()),
        Statement::TSGlobalDeclaration(_) => Some("global"),
        Statement::ExportDeclaration(e) => decl_name(&e.declaration),
        _ => None,
    };
    name.map(SmolStr::from)
}

struct Candidate {
    deps: TypeDeps,
    span: (usize, usize),
}

/// Byte spans (in the instance-script content) of the type declarations
/// to move to module scope, in source order.
pub(crate) fn hoisted_type_spans(program: &Program<'_>, ctx: &HoistContext) -> Vec<(usize, usize)> {
    let mut module_types = ctx
        .module_script
        .as_deref()
        .map(module_types)
        .unwrap_or_default();
    let mut disallowed_types: HashSet<SmolStr> = HashSet::new();
    let mut disallowed_values: HashSet<SmolStr> = HashSet::new();
    let mut interface_map: HashMap<SmolStr, Candidate> = HashMap::new();
    // Every top-level type declaration by name, for `$$Generic`
    // constraints (which move regardless of their dependencies).
    let mut all_types: Vec<(SmolStr, (usize, usize))> = Vec::new();

    for stmt in &program.body {
        let span = oxc_span::GetSpan::span(stmt);
        let span = (span.start as usize, span.end as usize);
        let decl: Option<&Declaration<'_>> = match stmt {
            Statement::ExportDeclaration(e) => Some(&e.declaration),
            _ => stmt.as_declaration(),
        };
        if let Statement::ImportDeclaration(import) = stmt {
            // Only a whole-clause `import type` shadows here; a
            // per-specifier `type` modifier does not (unlike in the
            // module script).
            if import.import_kind == ImportOrExportKind::Type {
                for spec in import.specifiers.iter().flatten() {
                    module_types.insert(SmolStr::from(spec.local().name.as_str()));
                }
            }
            continue;
        }
        let Some(decl) = decl else { continue };
        match decl {
            Declaration::TSInterfaceDeclaration(i) => {
                let name = SmolStr::from(i.id.name.as_str());
                all_types.push((name.clone(), span));
                let deps = crate::svelte2tsx_nodes::type_deps::collect_interface_deps(i);
                if module_types.contains(&name) {
                    disallowed_types.insert(name);
                } else {
                    interface_map.insert(name, Candidate { deps, span });
                }
            }
            Declaration::TSTypeAliasDeclaration(t) => {
                let name = SmolStr::from(t.id.name.as_str());
                all_types.push((name.clone(), span));
                let deps = crate::svelte2tsx_nodes::type_deps::collect_alias_deps(t);
                if module_types.contains(&name) {
                    disallowed_types.insert(name);
                } else {
                    interface_map.insert(name, Candidate { deps, span });
                }
            }
            Declaration::VariableDeclaration(v) => {
                for d in &v.declarations {
                    let mut names = Vec::new();
                    crate::process_instance_script_content::collect_binding_pattern_names(
                        &d.id, &mut names,
                    );
                    disallowed_values.extend(names);
                }
            }
            Declaration::FunctionDeclaration(f) => {
                if let Some(id) = &f.id {
                    disallowed_values.insert(SmolStr::from(id.name.as_str()));
                }
            }
            Declaration::ClassDeclaration(c) => {
                if let Some(id) = &c.id {
                    disallowed_values.insert(SmolStr::from(id.name.as_str()));
                }
            }
            Declaration::TSEnumDeclaration(e) => {
                disallowed_values.insert(SmolStr::from(e.id.name.as_str()));
            }
            Declaration::TSNamespaceDeclaration(n) => {
                disallowed_types.insert(SmolStr::from(n.id.name.as_str()));
                disallowed_values.insert(SmolStr::from(n.id.name.as_str()));
            }
            Declaration::TSGlobalDeclaration(_) => {
                disallowed_types.insert(SmolStr::new_static("global"));
                disallowed_values.insert(SmolStr::new_static("global"));
            }
            Declaration::TSExternalModuleDeclaration(_)
            | Declaration::TSImportEqualsDeclaration(_) => {}
        }
    }

    let mut out: Vec<(usize, usize)> = all_types
        .iter()
        .filter(|(name, _)| ctx.generic_constraints.iter().any(|c| c == name.as_str()))
        .map(|(_, span)| *span)
        .collect();

    disallowed_values.extend(ctx.accessed_stores.iter().cloned());

    let (props_name, props_deps) = match &ctx.props_type {
        PropsTypeShape::Untyped => (None, None),
        PropsTypeShape::Reference(root) => match interface_map.get(root) {
            Some(_) => (Some(root.clone()), None),
            None => (None, None),
        },
        PropsTypeShape::Inline(TypeDepsHandle(deps)) => (None, Some(deps)),
    };
    if props_name.is_some() || props_deps.is_some() {
        disallowed_types.extend(ctx.generic_names.iter().cloned());
        let is_allowed_reference = |disallowed_values: &HashSet<SmolStr>, r: &str| {
            !(disallowed_values.contains(r)
                || r == "$$props"
                || r == "$$restProps"
                || r == "$$slots"
                || (r.starts_with('$')
                    && !r[1..].starts_with('$')
                    && disallowed_values.contains(&r[1..])))
        };

        let mut hoistable: HashSet<SmolStr> = HashSet::new();
        // Visit in source order so the outcome is deterministic.
        let mut order: Vec<&SmolStr> = interface_map.keys().collect();
        order.sort_by_key(|n| interface_map[*n].span.0);
        let mut progress = true;
        while progress {
            progress = false;
            for name in &order {
                if hoistable.contains(*name) {
                    continue;
                }
                let deps = &interface_map[*name].deps;
                let mut can_hoist = true;
                let mut type_refs: Vec<&SmolStr> = deps.type_refs.iter().collect();
                type_refs.sort();
                for dep in type_refs {
                    if disallowed_types.contains(dep) {
                        disallowed_types.insert((*name).clone());
                        can_hoist = false;
                        break;
                    }
                    if interface_map.contains_key(dep) && !hoistable.contains(dep) {
                        can_hoist = false;
                    }
                }
                for dep in &deps.value_refs {
                    if !is_allowed_reference(&disallowed_values, dep) {
                        disallowed_types.insert((*name).clone());
                        can_hoist = false;
                        break;
                    }
                }
                if can_hoist {
                    hoistable.insert((*name).clone());
                    progress = true;
                }
            }
        }

        let props_hoistable = match (&props_name, props_deps) {
            (Some(name), _) => hoistable.contains(name),
            (None, Some(deps)) => deps
                .type_refs
                .iter()
                .chain(deps.value_refs.iter())
                .all(|d| {
                    !disallowed_types.contains(d) && is_allowed_reference(&disallowed_values, d)
                }),
            (None, None) => false,
        };
        if props_hoistable {
            out.extend(hoistable.iter().map(|n| interface_map[n].span));
        }
    }

    out.sort_unstable();
    out.dedup();
    out
}
