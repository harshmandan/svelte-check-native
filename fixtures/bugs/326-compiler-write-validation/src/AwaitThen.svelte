<script>
  let p = Promise.resolve('a');
</script>
{#await p then v}
  <input bind:value={v} />
{/await}
