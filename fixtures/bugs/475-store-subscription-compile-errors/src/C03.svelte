<script context="module">
	import { writable } from "svelte/store";
	const count = writable(0);
	console.log($count);
</script>
