<script context="module">
	import { writable } from "svelte/store";
	const count = writable(0);
	function f() { return $count; }
</script>
{$count}
