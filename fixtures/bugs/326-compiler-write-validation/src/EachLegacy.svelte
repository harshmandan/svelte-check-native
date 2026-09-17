<script>
  let items = ['a'];
</script>
{#each items as item}
  <input bind:value={item} />
{/each}
