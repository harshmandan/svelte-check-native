<script lang="ts">
    // v0.3 Item 6: `bind:clientWidth` inside `{#each as item, i}` —
    // the expression references the block-scoped iterator `i`, which
    // would be invisible if the emit wrote its contract check at the
    // top of __svn_tpl_check. Inline emit places it inside the
    // walker's per-block scope so `i` resolves.
    //
    // `items[i].width` is typed `number | undefined` (the width field
    // is optional). Must NOT fire — the assignment direction makes
    // `number | undefined` accept `number`.
    type Item = { id: string; width?: number }
    let items: Item[] = []
</script>

{#each items as _item, i}
    <div bind:clientWidth={items[i].width}>child</div>
{/each}
