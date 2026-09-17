<script>
  let v = $state('');
</script>
{#snippet row(p)}
  <input bind:value={p} />
{/snippet}
{@render row(v)}
