<script lang="ts">
	import { writable, type Writable } from "svelte/store";
	let page = $state({ customServer: writable("") as Writable<string> });
	let { customServer } = $derived(page);
	const other = $derived(writable(1));
</script>
<input bind:value={$customServer} />
<p>{$other}</p>
