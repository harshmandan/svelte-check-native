<script lang="ts">
let { rows, rowKey }: { rows: string[]; rowKey: (row: string) => string } = $props()
</script>

{#each rows as row (rowKey(row))}
  <li>{row}</li>
{/each}
