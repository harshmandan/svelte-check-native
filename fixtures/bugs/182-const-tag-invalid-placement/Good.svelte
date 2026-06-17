<script lang="ts">
	const items: { href: string; icon: any }[] = [];
</script>

{#each items as item (item.href)}
	{@const Icon = item.icon}
	<a href={item.href}>
		<Icon />{item.href}
	</a>
{/each}
