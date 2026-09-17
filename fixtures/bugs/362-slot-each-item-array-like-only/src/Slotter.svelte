<script lang="ts">
  export let items: Set<number>;
  export let list: number[];
</script>
{#each items as item}<slot {item} />{/each}
{#each list as entry}<slot name="list" {entry} />{/each}
