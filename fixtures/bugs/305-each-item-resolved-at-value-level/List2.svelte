<script lang="ts">
  type Item = { name: string };
  export let list: Item[] | undefined = undefined;
</script>
{#each list ?? [] as item}
  <slot {item} />
{/each}
