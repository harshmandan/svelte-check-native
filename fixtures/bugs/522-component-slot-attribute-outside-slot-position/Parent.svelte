<script lang="ts">
	import type { Snippet } from 'svelte'
	let { badge }: { badge?: Snippet } = $props()
</script>

{@render badge?.()}
