<svelte:options accessors={false} />
<script lang="ts">
  const x = true;
  export let value: number = 1;
</script>
{value}{x}
