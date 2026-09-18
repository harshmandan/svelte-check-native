<script>
	export let items;
</script>
{#each items as $state}
	<p>{$state}</p>
{/each}
