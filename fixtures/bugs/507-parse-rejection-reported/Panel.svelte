<script lang="ts">
	let { open, items }: { open: boolean; items: string[] } = $props();
</script>

{#if open}
	<div role="tabpanel">
	<ol>
		{#each items as item (item)}
			<li>{item}</li>
		{/each}
	</ol>
{/if}
