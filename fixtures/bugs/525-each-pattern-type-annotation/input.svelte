<script lang="ts">
	const items = ['a']
	const maybe: string[] | undefined = ['b']
	const pairs = [{ a: 1 }]
</script>

{#each items as item: string}
	<p>{item.toUpperCase()}</p>
{/each}
{#each pairs as { a }: { a: number }, i (a)}
	<p>{a + i}</p>
{/each}
{#each maybe! as maybe}
	<p>{maybe.length}</p>
{/each}
{#each items as item: number}
	<p>{item}</p>
{/each}
