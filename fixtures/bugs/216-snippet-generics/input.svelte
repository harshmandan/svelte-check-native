<script lang="ts">
    const nums = [1, 2, 3];
    const names = ['a', 'b'];
</script>

{#snippet row<T>(item: T, format: (v: T) => string)}
    <li>{format(item)}</li>
{/snippet}

{#snippet keyed<T extends { id: number }>(item: T)}
    <li>{item.id}</li>
{/snippet}

<ul>
    {@render row(nums[0], (n) => n.toFixed(2))}
    {@render row(names[0], (s) => s.toUpperCase())}
    {@render keyed(nums[0])}
</ul>
