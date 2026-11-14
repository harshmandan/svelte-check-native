<script lang="ts">
  let { items }: { items: string[] | undefined } = $props();
</script>

{#each items as item}
  <p>{item}</p>
{/each}
