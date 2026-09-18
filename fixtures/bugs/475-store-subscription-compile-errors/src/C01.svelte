<script>
	import { writable } from "svelte/store";
	export let stores = [writable(1)];
</script>
{#each stores as store}{$store}{/each}
