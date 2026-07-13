<script lang="ts">
    const items: { id: number; name: string }[] = [{ id: 1, name: 'a' }];
</script>

<ul>
    {#each items
        as item, i (item.id)}
        <li>{i}: {item.name}</li>
    {/each}
</ul>
