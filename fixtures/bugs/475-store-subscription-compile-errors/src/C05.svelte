<script>
	import { writable } from "svelte/store";
	const count = writable(0);
	function f(count) { return $count; }
</script>
