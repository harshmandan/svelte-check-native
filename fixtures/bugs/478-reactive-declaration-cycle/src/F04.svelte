<script>
	export let x = 0;
	let n = 0;
	$: n++, x;
	$: x = n;
</script>
