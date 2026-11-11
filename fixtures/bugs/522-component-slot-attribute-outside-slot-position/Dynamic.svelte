<script lang="ts">
	import Parent from './Parent.svelte'
	import Badge from './Badge.svelte'
</script>

<Parent>{#snippet badge()}<svelte:component this={Badge} slot="badge" />{/snippet}</Parent>
