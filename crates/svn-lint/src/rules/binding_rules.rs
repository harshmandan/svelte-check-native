//! Rules that loop over the scope tree's bindings post-walk.
//!
//! Unlike per-node rules, these fire at declaration sites based on
//! aggregated reference/reassignment data collected during the
//! scope-tree build. Running after the script-AST walk means
//! `ctx.scope_tree` is fully populated with
//! `binding.references` / `.reassigned` / `.mutated` flags.

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::scope::{BindingKind, RefParentKind, Reference, ScopeTree, is_rune_name};

/// Does `ref.ignored`'s snapshot include the rule's code? Port of
/// upstream's `ignore_map.get(node)?.some(codes => codes.has(code))`.
fn ref_ignores(r: &Reference, code: Code) -> bool {
    match &r.ignored {
        None => false,
        Some(list) => list.iter().any(|c| c == code.as_str()),
    }
}

/// Same check against the snapshot captured at the binding's
/// declaration site — for rules whose warning anchors on the
/// declaration rather than a reference.
fn binding_ignores(b: &crate::scope::Binding, code: Code) -> bool {
    match &b.ignored {
        None => false,
        Some(list) => list.iter().any(|c| c == code.as_str()),
    }
}

/// Pre-options pass: `store_rune_conflict` alone. Upstream fires it
/// from the store-sub synthesis loop, before even the
/// `<svelte:options>` attribute warnings.
pub fn visit_pre_options(ctx: &mut LintContext<'_>) {
    let tree = match ctx.scope_tree.take() {
        Some(t) => t,
        None => return,
    };
    store_rune_conflict(&tree, ctx);
    ctx.scope_tree = Some(tree);
}

/// Walk-time pass: rules whose upstream counterparts fire DURING the
/// instance-script walk. Their emissions are merged and sorted by
/// source position so interleaved anchors come out in walk order —
/// upstream does not group `state_referenced_locally` per binding.
pub fn visit(ctx: &mut LintContext<'_>) {
    // Take the tree out of the context so we can iterate its bindings
    // while still being able to `ctx.emit(...)`.
    let tree = match ctx.scope_tree.take() {
        Some(t) => t,
        None => return,
    };
    let mut pending: Vec<(svn_core::Range, Code, String)> = Vec::new();
    // reactive_declaration_module_script_dependency fires in non-runes
    // mode too (legacy reactivity), so it runs regardless.
    reactive_declaration_module_script_dependency(&tree, &mut pending);
    custom_element_props_identifier(&tree, ctx, &mut pending);
    if ctx.runes {
        state_referenced_locally(&tree, &mut pending);
    }
    pending.sort_by_key(|(range, _, _)| range.start);
    for (range, code, msg) in pending {
        ctx.emit(code, msg, range);
    }
    // Put it back — downstream template walkers may still want to
    // query the tree (element_rules has already run by this point,
    // but keep the invariant so future rules don't trip over None).
    ctx.scope_tree = Some(tree);
}

/// Post-template pass: upstream runs these as loops over scope
/// declarations AFTER all three walks (module, instance, template),
/// so they emit after template warnings.
pub fn visit_post_template(ctx: &mut LintContext<'_>) {
    let tree = match ctx.scope_tree.take() {
        Some(t) => t,
        None => return,
    };
    if ctx.runes {
        non_reactive_update(&tree, ctx);
    } else {
        export_let_unused(&tree, ctx);
    }
    ctx.scope_tree = Some(tree);
}

/// Upstream: `visitors/Identifier.js:154-160`. For every module-scope
/// binding that's been reassigned (e.g. by an `update()` function),
/// fire at each reference inside an instance-script `$:` reactive
/// statement — the reactivity system doesn't observe module-level
/// writes.
fn reactive_declaration_module_script_dependency(
    tree: &ScopeTree,
    pending: &mut Vec<(svn_core::Range, Code, String)>,
) {
    for (_, binding) in tree.all_bindings() {
        if binding.scope != tree.module_root || !binding.reassigned {
            continue;
        }
        for r in &binding.references {
            if r.in_reactive_statement
                && !ref_ignores(r, Code::reactive_declaration_module_script_dependency)
            {
                let msg = messages::reactive_declaration_module_script_dependency();
                pending.push((
                    r.range,
                    Code::reactive_declaration_module_script_dependency,
                    msg,
                ));
            }
        }
    }
}

/// Upstream: `2-analyze/index.js:400-407`. Fires when a `$NAME(...)`
/// call looks like a rune BUT there's a local `NAME` binding — the
/// $-prefix creates a store subscription instead, which shadows the
/// rune. Fires at each call-position reference of the synthesized
/// `$NAME` store-sub binding.
fn store_rune_conflict(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
    for (_, binding) in tree.all_bindings() {
        if binding.kind != BindingKind::StoreSub {
            continue;
        }
        let name = binding.name.as_str();
        if !is_rune_name(name) {
            continue;
        }
        let store_name = &name[1..];
        // Only fire when the backing identifier actually exists — a
        // plain misspelled rune (no `state` declaration anywhere)
        // isn't a conflict, it's an unknown reference.
        let backing_exists = tree.resolve(tree.module_root, store_name).is_some()
            || tree.resolve(tree.instance_root, store_name).is_some();
        if !backing_exists {
            continue;
        }
        for r in &binding.references {
            if !r.parent_is_call {
                continue;
            }
            // Deliberately NOT consulting `r.ignored`: upstream fires
            // this warning from the store-sub synthesis loop, BEFORE
            // the analyze walk populates the ignore map — neither a
            // script `// svelte-ignore` nor a template comment can
            // suppress it (verified against the compiler).
            let msg = messages::store_rune_conflict(store_name);
            ctx.emit(Code::store_rune_conflict, msg, r.range);
        }
    }
}

/// Upstream: `visitors/Identifier.js:108-151`. Fires when a runes-mode
/// `$state` / `$state.raw` / `$derived` / `$props` rest-prop binding
/// is read at the same `function_depth` as its declaration — i.e.
/// outside a closure or `$derived(...)` arg. The `$state` subcase
/// only fires when the binding is also reassigned OR its initial
/// argument is a primitive (uses the conservative `should_proxy`
/// analog in `scope.rs::is_primitive_expr`).
fn state_referenced_locally(tree: &ScopeTree, pending: &mut Vec<(svn_core::Range, Code, String)>) {
    for (_, binding) in tree.all_bindings() {
        // Upstream gate (visitors/Identifier.js:110-119): fires on
        // `state` (specific reassigned / primitive-init) /
        // `raw_state` / `derived` / `prop` / `rest_prop`, version-
        // gated on `state_locally_fires_on_props` (svelte@5.45.3,
        // PR #17266) and `state_locally_rest_prop` (svelte@5.51.2,
        // PR #17708).
        //
        // The whole gate is pre-computed at scope-build time into
        // `binding.fires_state_referenced_locally` so the rule stays
        // a simple predicate read. See `scope::populate_compat_gated_fields`.
        if !binding.fires_state_referenced_locally {
            continue;
        }
        let binding_depth = tree.scope(binding.scope).function_depth;

        for r in &binding.references {
            // Skip writes.
            if matches!(
                r.parent_kind,
                RefParentKind::AssignmentLeft | RefParentKind::UpdateTarget
            ) {
                continue;
            }
            if r.function_depth_at_use != binding_depth {
                continue;
            }
            if ref_ignores(r, Code::state_referenced_locally) {
                continue;
            }
            let type_var = if r.nested_in_state_call {
                "derived"
            } else {
                "closure"
            };
            let msg = messages::state_referenced_locally(binding.name.as_str(), type_var);
            pending.push((r.range, Code::state_referenced_locally, msg));
        }
    }
}

/// Upstream: `2-analyze/index.js:744-778`. For every normal-kind
/// binding that's been reassigned, fire once if at least one of its
/// references lives in the template and is NOT captured by a
/// function closure. The `bind:this={…}` subcase fires only when the
/// bind:this site is nested inside a control-flow block
/// (`{#if}`/`{#each}`/`{#await}`/`{#key}`) — otherwise a ref-capture
/// pattern doesn't need reactivity.
fn non_reactive_update(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
    for (_, binding) in tree.all_bindings() {
        if binding.kind != BindingKind::Normal || !binding.reassigned {
            continue;
        }
        for r in &binding.references {
            if !r.in_template {
                continue;
            }
            if r.in_function_closure {
                continue;
            }
            if r.is_bind_this && !r.in_control_flow {
                continue;
            }
            // Honour `// svelte-ignore non_reactive_update` leading
            // the declaration — the snapshot captured when the
            // binding was declared (upstream: `ignore_map` at the
            // declaration node).
            if binding_ignores(binding, Code::non_reactive_update) {
                break;
            }
            let msg = messages::non_reactive_update(binding.name.as_str());
            ctx.emit(Code::non_reactive_update, msg, binding.range);
            break;
        }
    }
}

/// Upstream: `2-analyze/index.js:800-814`. Non-runes-mode loop —
/// for each prop-style binding in the instance scope, fire if it
/// has no references outside its own declaration (and no store-sub
/// counterpart named `$NAME`). References inside `ExportSpecifier`
/// (the local side of `export { x as y }`) don't count as "usage"
/// since they're just re-exports.
fn export_let_unused(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
    for (_, binding) in tree.all_bindings() {
        if binding.scope != tree.instance_root {
            continue;
        }
        if !matches!(binding.kind, BindingKind::Prop | BindingKind::BindableProp) {
            continue;
        }
        if binding.name.as_str().starts_with("$$") {
            continue;
        }
        // A synthetic store-sub named `$NAME` counts as usage.
        let dollar = format!("${}", binding.name);
        if tree.resolve(tree.instance_root, &dollar).is_some() {
            continue;
        }
        if binding.references.is_empty() {
            if binding_ignores(binding, Code::export_let_unused) {
                continue;
            }
            let msg = messages::export_let_unused(binding.name.as_str());
            ctx.emit(Code::export_let_unused, msg, binding.range);
        }
    }
}

/// Upstream: `VariableDeclarator.js:72-83`. When the file declares
/// `<svelte:options customElement={…}>` AND the option expression
/// doesn't include a `props` key, every `$props()` declaration that
/// uses the Identifier (`let props = $props()`) or rest-element
/// (`let { ...props } = $props()`) form emits a warning. The
/// candidates are collected by the scope walker so this step just
/// filters by the options gate. Scope-walker snapshot of the ignore
/// stack is honoured per-candidate.
fn custom_element_props_identifier(
    tree: &ScopeTree,
    ctx: &LintContext<'_>,
    pending: &mut Vec<(svn_core::Range, Code, String)>,
) {
    let Some(info) = ctx.custom_element_info.clone() else {
        return;
    };
    if info.has_props_option {
        return;
    }
    for (i, range) in tree.custom_element_props_candidates.iter().enumerate() {
        if let Some(Some(codes)) = tree.custom_element_props_ignored.get(i)
            && codes
                .iter()
                .any(|c| c == Code::custom_element_props_identifier.as_str())
        {
            continue;
        }
        let msg = messages::custom_element_props_identifier();
        pending.push((*range, Code::custom_element_props_identifier, msg));
    }
}
