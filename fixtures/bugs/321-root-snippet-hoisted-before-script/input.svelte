<script lang="ts">
  import type { Snippet } from 'svelte';
  let { header = defaultHeader }: { header?: Snippet<[string]> } = $props();
</script>
{#snippet defaultHeader(n: number)}<h1>{n}</h1>{/snippet}
{@render header('x')}
