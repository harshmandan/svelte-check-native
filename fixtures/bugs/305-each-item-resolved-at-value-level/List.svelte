<script lang="ts">
  type Item = { name: string };
  export let list: (Item | null)[] = [];
</script>
{#each list.filter((x): x is Item => x !== null) as item}
  <slot {item} />
{/each}
