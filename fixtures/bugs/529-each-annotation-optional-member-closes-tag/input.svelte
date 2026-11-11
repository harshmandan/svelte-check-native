<script lang="ts">
	const rows: { b?: number }[] = []
</script>

{#each rows as row: { b?: number }, i}
	{row.b}{i}
{/each}
