<script>
  let items = $state(['a']);
</script>
{#each items as item, i}
  <input bind:value={i} />
{/each}
