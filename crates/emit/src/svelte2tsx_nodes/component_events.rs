//! Component event-shape extraction (`$$Events` interface, typed
//! `createEventDispatcher<T>()`, `on:event` directive collection).
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ComponentEvents.ts`.
//!
//! Upstream's `ComponentEvents` class concentrates the whole concern.
//! Ours is split across emit phases because each phase touches a
//! different layer of the overlay:
//!
//! - **`rewrite_dispatcher_typing` (this file)** — script-body
//!   rewrite. Walks the parsed instance script for top-level
//!   `const X = createEventDispatcher()` (no type args) and splices
//!   `<__SvnCustomEvents<$$Events>>` after the callee identifier so
//!   subsequent `dispatch('name', detail)` calls check against the
//!   declared `$$Events` interface. Mirrors `ComponentEvents.ts:130-148`.
//!
//! - **Per-component event collection** —
//!   `analyze::nodes::inline_component` captures each `on:NAME={handler}`
//!   directive at instantiation sites into
//!   `ComponentInstantiation::on_events`.
//!
//! - **Emit** of the `$inst.$on("NAME", (handler))` calls —
//!   [`crate::nodes::inline_component::emit_on_event_calls`].
//!
//! - **Default-export Events surface** (`{ [K in keyof $$Events]:
//!   CustomEvent<$$Events[K]> }` mapped type or the lax
//!   `{ [evt: string]: CustomEvent<any> }` fallback) — built in
//!   [`crate::render_function::emit_render_body_return`].
//!
//! - **$$Events detection** — the `<script strictEvents>` opt-in
//!   and Svelte-5 runes-mode triggers live in
//!   [`crate::svelte4::compat::has_strict_events_ast`],
//!   [`crate::svelte4::compat::has_strict_events_attr`], and
//!   [`crate::svelte4::compat::is_runes_mode`].
//!
//! No typed-`createEventDispatcher<T>()` extraction: we don't yet
//! parse the Svelte-5-style typed dispatcher's generic argument back
//! into a `$$Events`-shaped projection. A child using the typed
//! dispatcher without a `$$Events` interface routes through the lax
//! `[evt: string]: CustomEvent<any>` overload (matches v0.2.5
//! behaviour; tracked as a lint-only follow-up in `notes/`).

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Expression, Statement};
use svn_analyze::{WalkNode, collect_ctor_locals, walk_statement_descend};
use svn_parser::{ScriptLang, parse_script_body};

/// Walk top-level `const X = createEventDispatcher()` declarators and
/// return `content` with `<__SvnCustomEvents<$$Events>>` spliced in
/// after each untyped dispatcher call's callee identifier. When no
/// untyped dispatcher is found (or no `interface $$Events` is
/// declared, per the caller's `should_rewrite` gate), returns
/// `content` unchanged.
pub(crate) fn rewrite_dispatcher_typing(content: &str, lang: ScriptLang) -> String {
    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, content, lang);

    let ctor_locals = collect_ctor_locals(&parsed.program);
    if ctor_locals.is_empty() {
        return content.to_string();
    }

    let mut insertions: Vec<(usize, &'static str)> = Vec::new();
    // Walk recursively so nested untyped dispatchers (inside function
    // bodies, control-flow blocks, callback args, for-init slots)
    // also get the typed-events rewrite. Driven by walk_statement_
    // descend — the closure pattern-matches on WalkNode::{Statement(
    // VariableDeclaration | ExportNamedDeclaration), ForInitVarDecl}
    // and records each untyped call's callee.span.end byte position.
    let handle_var_decl = |decl: &oxc_ast::ast::VariableDeclaration<'_>,
                           out: &mut Vec<(usize, &'static str)>| {
        for declarator in &decl.declarations {
            if !matches!(&declarator.id, BindingPattern::BindingIdentifier(_)) {
                continue;
            }
            let Some(init) = &declarator.init else {
                continue;
            };
            if let Expression::CallExpression(call) = init
                && let Expression::Identifier(callee_id) = &call.callee
                && ctor_locals.contains(callee_id.name.as_str())
                && call.type_arguments.is_none()
            {
                out.push((callee_id.span.end as usize, "<__SvnCustomEvents<$$Events>>"));
            }
        }
    };
    for stmt in &parsed.program.body {
        walk_statement_descend(stmt, &mut |node| match node {
            WalkNode::Statement(Statement::VariableDeclaration(decl)) => {
                handle_var_decl(decl, &mut insertions);
            }
            WalkNode::Statement(Statement::ExportNamedDeclaration(ed)) => {
                if let Some(oxc_ast::ast::Declaration::VariableDeclaration(decl)) = &ed.declaration
                {
                    handle_var_decl(decl, &mut insertions);
                }
            }
            WalkNode::ForInitVarDecl(decl) => handle_var_decl(decl, &mut insertions),
            _ => {}
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    fn ts(src: &str) -> String {
        rewrite_dispatcher_typing(src, ScriptLang::Ts)
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
