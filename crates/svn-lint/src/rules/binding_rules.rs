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
use crate::scope::{BindingKind, InitialKind, Reference, RefParentKind, ScopeTree, is_rune_name};

/// Does `ref.ignored`'s snapshot include the rule's code? Port of
/// upstream's `ignore_map.get(node)?.some(codes => codes.has(code))`.
fn ref_ignores(r: &Reference, code: Code) -> bool {
    match &r.ignored {
        None => false,
        Some(list) => list.iter().any(|c| c == code.as_str()),
    }
}

/// Driver: run every binding-driven rule. Called once per file
/// from `walk::walk` after `ctx.scope_tree` is populated.
pub fn visit(ctx: &mut LintContext<'_>) {
    // Take the tree out of the context so we can iterate its bindings
    // while still being able to `ctx.emit(...)`.
    let tree = match ctx.scope_tree.take() {
        Some(t) => t,
        None => return,
    };
    // reactive_declaration_module_script_dependency fires in non-runes
    // mode too (legacy reactivity), so it runs regardless.
    reactive_declaration_module_script_dependency(&tree, ctx);
    store_rune_conflict(&tree, ctx);
    bind_invalid_each_rest(&tree, ctx);
    custom_element_props_identifier(&tree, ctx);
    if ctx.runes {
        state_referenced_locally(&tree, ctx);
        non_reactive_update(&tree, ctx);
    } else {
        export_let_unused(&tree, ctx);
    }
    // Put it back — downstream template walkers may still want to
    // query the tree (element_rules has already run by this point,
    // but keep the invariant so future rules don't trip over None).
    ctx.scope_tree = Some(tree);
}

/// Upstream: `visitors/Identifier.js:154-160`. For every module-scope
/// binding that's been reassigned (e.g. by an `update()` function),
/// fire at each reference inside an instance-script `$:` reactive
/// statement — the reactivity system doesn't observe module-level
/// writes.
fn reactive_declaration_module_script_dependency(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
    for (_, binding) in tree.all_bindings() {
        if binding.scope != tree.module_root || !binding.reassigned {
            continue;
        }
        for r in &binding.references {
            if r.in_reactive_statement
                && !ref_ignores(r, Code::reactive_declaration_module_script_dependency)
            {
                let msg = messages::reactive_declaration_module_script_dependency();
                ctx.emit(
                    Code::reactive_declaration_module_script_dependency,
                    msg,
                    r.range,
                );
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
            if ref_ignores(r, Code::store_rune_conflict) {
                continue;
            }
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
fn state_referenced_locally(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
    for (_, binding) in tree.all_bindings() {
        // Upstream gate (visitors/Identifier.js:110-119): fires on
        // `state` (specific reassigned / primitive-init) /
        // `raw_state` / `derived` / `prop` / `rest_prop`. Two gates
        // are version-dependent:
        //
        // - `prop` / `bindable_prop` — added in svelte@5.45.3
        //   (PR #17266). Pre-5.45.3, only state / derived fire;
        //   reading a regular destructured prop at top-level didn't
        //   warn. Gated by `compat.state_locally_fires_on_props`.
        // - `rest_prop` — added in svelte@5.51.2 (PR #17708). Gated
        //   by `compat.state_locally_rest_prop` (which implies
        //   `state_locally_fires_on_props`).
        let reactive_kind = match binding.kind {
            BindingKind::RawState | BindingKind::Derived => true,
            BindingKind::Prop => ctx.compat.state_locally_fires_on_props,
            BindingKind::RestProp => {
                ctx.compat.state_locally_fires_on_props && ctx.compat.state_locally_rest_prop
            }
            BindingKind::State => {
                binding.reassigned || primitive_initial(&binding.initial)
            }
            _ => false,
        };
        if !reactive_kind {
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
            let type_var = if r.nested_in_state_call { "derived" } else { "closure" };
            let msg = messages::state_referenced_locally(binding.name.as_str(), type_var);
            ctx.emit(Code::state_referenced_locally, msg, r.range);
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
            // Honour `// svelte-ignore non_reactive_update` immediately
            // preceding the declaration. Template `<!-- svelte-ignore -->`
            // is handled by the LintContext ignore-stack; script
            // comments aren't on that stack so we do the lookup here.
            if has_script_leading_ignore(
                ctx.source,
                binding.range.start,
                Code::non_reactive_update.as_str(),
                ctx.runes,
            ) {
                break;
            }
            let msg = messages::non_reactive_update(binding.name.as_str());
            ctx.emit(Code::non_reactive_update, msg, binding.range);
            break;
        }
    }
}

/// Scan the lines preceding `decl_start` for `// svelte-ignore CODE`
/// comments mentioning `code`. A blank line between the comment and
/// the declaration breaks the chain (matches upstream's trim-based
/// `extract_svelte_ignore` behaviour at the statement level).
fn has_script_leading_ignore(source: &str, decl_start: u32, code: &str, runes: bool) -> bool {
    // Walk to the start of the line containing `decl_start`.
    let bytes = source.as_bytes();
    let mut line_start = decl_start as usize;
    while line_start > 0 && bytes[line_start - 1] != b'\n' {
        line_start -= 1;
    }
    // Iterate preceding lines in reverse.
    loop {
        if line_start == 0 {
            return false;
        }
        // Line before the current one: [prev_start, line_start - 1]
        // (excluding the trailing \n).
        let mut prev_end = line_start - 1;
        if prev_end > 0 && bytes[prev_end] == b'\r' {
            // skip \r of \r\n
            prev_end -= 1;
        }
        let mut prev_start = prev_end;
        while prev_start > 0 && bytes[prev_start - 1] != b'\n' {
            prev_start -= 1;
        }
        let raw = &source[prev_start..prev_end + 1];
        let trimmed = raw.trim_start();
        if trimmed.is_empty() {
            // Blank line — chain broken.
            return false;
        }
        if let Some(rest) = trimmed.strip_prefix("//")
            && let Some(body) = rest.trim_start().strip_prefix("svelte-ignore")
            && body.chars().next().is_some_and(char::is_whitespace)
        {
            let codes = crate::ignore::parse_ignore_codes_public(body.trim_start(), runes);
            if codes.iter().any(|c| c == code) {
                return true;
            }
            // svelte-ignore line but different code — keep scanning.
            line_start = prev_start;
            continue;
        }
        return false;
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
            if has_script_leading_ignore(
                ctx.source,
                binding.range.start,
                Code::export_let_unused.as_str(),
                ctx.runes,
            ) {
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
fn custom_element_props_identifier(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
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
        ctx.emit(Code::custom_element_props_identifier, msg, *range);
    }
}

/// Upstream: `visitors/BindDirective.js:271`. Each-block bindings
/// declared inside a rest element produce a fresh object on every
/// iteration, so `bind:*` writes never reach the original. Fires at
/// the binding declaration (name inside the `...rest` pattern).
fn bind_invalid_each_rest(tree: &ScopeTree, ctx: &mut LintContext<'_>) {
    for (_, binding) in tree.all_bindings() {
        if binding.kind != BindingKind::Each || !binding.inside_rest {
            continue;
        }
        if !binding.has_bind_reference {
            continue;
        }
        let msg = messages::bind_invalid_each_rest(binding.name.as_str());
        ctx.emit(Code::bind_invalid_each_rest, msg, binding.range);
    }
}

/// Was this binding declared with a `$state(primitive)`-style init?
/// The `InitialKind::RuneCall.primitive_arg` flag captures this —
/// true for `$state(0)`, `$state.raw(0)`, false for `$state({})` and
/// friends. For non-rune inits we return `false` (the check only
/// applies in the `State` kind branch upstream).
fn primitive_initial(init: &InitialKind) -> bool {
    matches!(init, InitialKind::RuneCall { primitive_arg: true, .. })
}
