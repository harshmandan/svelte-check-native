<script>
  let items = $state(['a']);
</script>
{#each items as item}
  <input bind:value={item} />
{/each}
