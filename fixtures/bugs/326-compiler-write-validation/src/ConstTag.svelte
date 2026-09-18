<script>
  let items = $state(['a']);
</script>
{#each items as item}
  {@const up = item.toUpperCase()}
  <input bind:value={up} />
{/each}
