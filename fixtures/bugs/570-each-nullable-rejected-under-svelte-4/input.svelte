<script lang="ts">
  export let items: string[] | undefined;
</script>

{#each items as item}
  <p>{item}</p>
{/each}
