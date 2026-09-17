<script lang="ts">
  let { n = 0 }: { n?: number } = $props();
</script>
{#if n}<svelte:self n={n - 1} class:a={n} />{/if}
