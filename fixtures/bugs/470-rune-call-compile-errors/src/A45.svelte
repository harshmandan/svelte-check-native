<script>
	let x = $state(0);
	$effect(() => { $inspect.trace("x"); });
	const g = function () { "use strict"; $inspect.trace(); };
</script>
