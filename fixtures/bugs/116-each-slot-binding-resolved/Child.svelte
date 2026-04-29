<script lang="ts">
    // Child renders one slot per row of `items`. The slot exposes
    // `item` (the row) and `index` (the iteration counter). Pre-fix
    // these were silently dropped from the slot-def because
    // `{item}` and `{index}` reference template-shadowed bindings
    // that the resolver couldn't rewrite. Stage 2 of the
    // SlotHandler port now resolves them through the each-block's
    // outer expression: `item` types as the array element, `index`
    // types as `number`.
    export let items: { id: string; label: string }[] = []
</script>

{#each items as item, index}
    <slot {item} {index} />
{/each}
