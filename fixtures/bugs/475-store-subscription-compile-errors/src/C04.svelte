<script module>
	import { writable } from "svelte/store";
	const count = writable(0);
</script>
<script>
	let x = $state(0);
	console.log($count);
</script>
