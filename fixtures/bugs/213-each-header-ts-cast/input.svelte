<script lang="ts">
    interface Item { name: string }
    const items: unknown = [{ name: 'a' }];
</script>

<ul>
    {#each items as unknown as Item[] as item}
        <li>{item.name}</li>
    {/each}
</ul>
