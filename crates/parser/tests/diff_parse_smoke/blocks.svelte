<script>
    let { items = [], promise } = $props();
</script>

{#if items.length}
    {#each items as item, i (item.id)}
        <span data-i={i}>{item.name}</span>
    {:else}
        empty
    {/each}
{:else if promise}
    {#await promise then value}
        {value}
    {:catch err}
        {err}
    {/await}
{:else}
    nothing
{/if}

{#snippet row(cells)}
    {#key cells}
        {@render row(cells)}
    {/key}
{/snippet}
