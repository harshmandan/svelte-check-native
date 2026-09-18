<script>
	export let x = 0;
	$: [p, q] = [x, 1];
	$: x = p + q;
</script>
{p}
