<script module lang="ts">
  export { row };
</script>
<script lang="ts">
  let count = $state(0);
</script>
{#snippet row(n: number)}<b>{n}</b>{/snippet}
{@render row(count)}
