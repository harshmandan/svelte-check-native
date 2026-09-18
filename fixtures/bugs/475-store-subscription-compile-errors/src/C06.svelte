<script>
	import { writable } from "svelte/store";
	const count = writable(0);
</script>
{#each [1] as count}{$count}{/each}
