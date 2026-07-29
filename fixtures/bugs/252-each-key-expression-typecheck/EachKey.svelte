<script lang="ts">
let { items }: { items: { id: number }[] } = $props()
function keyOf(x: string): string { return x }
</script>

{#each items as item (keyOf(item.id))}
  <li>{item.id}</li>
{/each}
