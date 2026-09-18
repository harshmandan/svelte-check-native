<script>
	export let x = 0;
	let y = 0;
	$: y += x;
	$: x = y;
</script>
