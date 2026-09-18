<script>
	let { items } = $props();
</script>
{#each items as item}
	{@const a = item}
	{#snippet a()}x{/snippet}
{/each}
