<script lang="ts">
    // Round-12 follow-up #3: slot-attr expressions whose CALLBACK
    // bodies close over an outer template local. Pre-fix native
    // skipped arrow/function bodies entirely in
    // `slot_attr_rewrite::walk_value_expr`, so a slot attr like
    // `<slot value={items.map(x => row.id)}>` where `row` is a
    // template-local from `{#each rows as row}` left `row`
    // untouched in the emit. The reference then leaked to module
    // scope (failing TS2304 if no module-scope `row` existed, or
    // resolving to the wrong binding).
    //
    // Native now walks arrow/function bodies with a shadow stack:
    // the function's params are pushed before recursing, so
    // identifiers whose name MATCHES a param are left alone, but
    // closed-over outer template locals (like `row`) get rewritten.
    type Row = { id: number; vals: number[] }
    let { rows }: { rows: Row[] } = $props()
</script>

{#each rows as row}
    <slot doubled={row.vals.map(x => x + row.id)} />
{/each}
