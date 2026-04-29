<script lang="ts">
    // Reviewer follow-up #6: a named-slot consumer child of
    // `<svelte:component>` / `<svelte:self>` must destructure from the
    // PARENT component's `$$slot_def["X"]`. Pre-fix the special-
    // element path passed only `has_let_bindings` (false here, since
    // there's no top-level `let:` on the svelte:component itself) and
    // walked children via plain `emit_template_node` instead of
    // `walk_child_with_slot_let` — the parent instance never got
    // hoisted, the named-slot wrapper destructure never landed, and
    // the inner `let:title` binding referenced an undeclared name
    // inside the child fragment.
    //
    // Post-fix the path pre-scans children for
    // `child_is_slot_let_consumer`, hoists the parent inst when any
    // exists, and routes through `walk_child_with_slot_let` so the
    // consumer wrapper destructures against the synthetic
    // component's `$$slot_def["header"]`.
    import Child from './Child.svelte'
    const Dynamic = Child
</script>

<svelte:component this={Dynamic}>
    <span slot="header" let:title>{title}</span>
</svelte:component>
