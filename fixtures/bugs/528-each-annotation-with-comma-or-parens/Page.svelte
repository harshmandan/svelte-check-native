<script lang="ts">
	const entries: Map<string, number>[] = []
	const handlers: ((a: string) => void)[] = []
</script>

{#each entries as entry: Map<string, number>}
	{entry.size}
{/each}
{#each handlers as handler: (a: string) => void, i}
	{handler(i)}
{/each}
