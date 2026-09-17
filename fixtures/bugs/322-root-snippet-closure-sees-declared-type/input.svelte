<script lang="ts">
  type Player = { play(): void };
  let icon: Player | null = $state(null);
  let count = $state(0);
</script>
{#snippet row()}
  <button onclick={() => icon?.play()}>{count}</button>
{/snippet}
{@render row()}
