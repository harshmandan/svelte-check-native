//! Reviewer follow-up #3b: rewrite untyped `createEventDispatcher()`
//! calls to `createEventDispatcher<__SvnCustomEvents<$$Events>>()`
//! when an `interface $$Events` (or `type $$Events`) is declared.
//!
//! Without the rewrite, `dispatch('name', detail)` calls inside the
//! component go un-checked: the dispatcher's signature is generic
//! over its `<T>` arg, but with no `<T>` supplied TS infers `{}` as
//! the detail-map and any first arg passes. Upstream svelte2tsx's
//! `ComponentEvents.ts:130` does the same rewrite.
//!
//! Mirrors the existing `state_nullish_rewrite` shape: walk the
//! parsed AST for top-level `const X = createEventDispatcher()` (with
//! NO type arguments), record byte-positions of insertion points,
//! splice in `<__SvnCustomEvents<$$Events>>` after the call's
//! callee identifier so the call becomes
//! `createEventDispatcher<__SvnCustomEvents<$$Events>>()`.
//!
//! Aliased imports (`import { createEventDispatcher as ced }`) are
//! resolved via `find_typed_dispatcher_local_names`-style ctor-locals
//! inference; non-Svelte imports / local functions named
//! `createEventDispatcher` are excluded by the same gate that
//! `find_dispatcher_local_names` / `find_typed_dispatcher_local_names`
//! use (only `from 'svelte'` counts).

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Expression, Statement};
use svn_parser::{ScriptLang, parse_script_body};

/// Walk top-level `const X = createEventDispatcher()` declarators and
/// return `content` with `<__SvnCustomEvents<$$Events>>` spliced in
/// after each untyped dispatcher call's callee identifier. When no
/// untyped dispatcher is found (or no `interface $$Events` is
/// declared, per the caller's `should_rewrite` gate), returns
/// `content` unchanged.
pub fn rewrite(content: &str, lang: ScriptLang) -> String {
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, content, lang);

    let ctor_locals = collect_ctor_locals(&parsed.program);
    if ctor_locals.is_empty() {
        return content.to_string();
    }

    let mut insertions: Vec<(usize, &'static str)> = Vec::new();
    for stmt in &parsed.program.body {
        let Statement::VariableDeclaration(decl) = stmt else {
            continue;
        };
        for declarator in &decl.declarations {
            // Binding identifier (`const X = …`) — destructures /
            // patterns aren't dispatcher bindings.
            if !matches!(&declarator.id, BindingPattern::BindingIdentifier(_)) {
                continue;
            }
            let Some(init) = &declarator.init else {
                continue;
            };
            let Expression::CallExpression(call) = init else {
                continue;
            };
            let Expression::Identifier(callee_id) = &call.callee else {
                continue;
            };
            if !ctor_locals.iter().any(|n| n == callee_id.name.as_str()) {
                continue;
            }
            // Already typed — don't double-specify.
            if call.type_arguments.is_some() {
                continue;
            }
            insertions.push((
                callee_id.span.end as usize,
                "<__SvnCustomEvents<$$Events>>",
            ));
        }
    }

    if insertions.is_empty() {
        return content.to_string();
    }
    // Reverse-sort by position so later insertions don't shift
    // earlier ones.
    insertions.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
    let mut out = content.to_string();
    for (pos, text) in insertions {
        out.insert_str(pos, text);
    }
    out
}

/// Same shape as `crates/analyze/src/props.rs::collect_ctor_locals`,
/// inlined here to avoid pulling the analyze crate into the
/// rewrite path. Only imports whose source is exactly `'svelte'`
/// count — local functions and non-Svelte imports named
/// `createEventDispatcher` don't trigger the rewrite.
fn collect_ctor_locals(program: &oxc_ast::ast::Program<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for stmt in &program.body {
        let Statement::ImportDeclaration(decl) = stmt else {
            continue;
        };
        if decl.source.value.as_str() != "svelte" {
            continue;
        }
        let Some(specifiers) = &decl.specifiers else {
            continue;
        };
        for spec in specifiers {
            let oxc_ast::ast::ImportDeclarationSpecifier::ImportSpecifier(s) = spec else {
                continue;
            };
            let imported = match &s.imported {
                oxc_ast::ast::ModuleExportName::IdentifierName(n) => n.name.as_str(),
                oxc_ast::ast::ModuleExportName::IdentifierReference(r) => r.name.as_str(),
                oxc_ast::ast::ModuleExportName::StringLiteral(l) => l.value.as_str(),
            };
            if imported == "createEventDispatcher" {
                out.push(s.local.name.to_string());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        rewrite(src, ScriptLang::Ts)
    }

    #[test]
    fn rewrites_untyped_dispatcher() {
        let src = "import { createEventDispatcher } from 'svelte';\n\
                   const dispatch = createEventDispatcher();";
        assert_eq!(
            ts(src),
            "import { createEventDispatcher } from 'svelte';\n\
             const dispatch = createEventDispatcher<__SvnCustomEvents<$$Events>>();"
        );
    }

    #[test]
    fn leaves_typed_dispatcher_alone() {
        let src = "import { createEventDispatcher } from 'svelte';\n\
                   const dispatch = createEventDispatcher<{ foo: string }>();";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn skips_local_function_with_same_name() {
        let src = "function createEventDispatcher() { return null; }\n\
                   const d = createEventDispatcher();";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn skips_non_svelte_import() {
        let src = "import { createEventDispatcher } from 'some-other-pkg';\n\
                   const d = createEventDispatcher();";
        assert_eq!(ts(src), src);
    }

    #[test]
    fn handles_aliased_import() {
        let src = "import { createEventDispatcher as ced } from 'svelte';\n\
                   const d = ced();";
        assert_eq!(
            ts(src),
            "import { createEventDispatcher as ced } from 'svelte';\n\
             const d = ced<__SvnCustomEvents<$$Events>>();"
        );
    }
}
