<script lang="ts">
	import Parent from './Parent.svelte'
	import Badge from './Badge.svelte'
	let show = $state(true)
</script>

<Parent>
	{#snippet badge()}
		<Badge slot="badge" />
	{/snippet}
</Parent>
<Parent><div><Badge slot="badge" /></div></Parent>
<Badge slot="badge" />
<Parent><svelte:boundary><Badge slot="badge" /></svelte:boundary></Parent>
<Parent>{#snippet badge()}<Badge slot="badge">hi</Badge>{/snippet}</Parent>
<Parent>{#if show}<Badge slot="badge" />{/if}</Parent>
