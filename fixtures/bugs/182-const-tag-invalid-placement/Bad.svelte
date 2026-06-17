<script lang="ts">
	const items: { href: string; icon: any }[] = [];
</script>

{#each items as item (item.href)}
	<a href={item.href}>
		{@const Icon = item.icon}
		<Icon />{item.href}
	</a>
{/each}
