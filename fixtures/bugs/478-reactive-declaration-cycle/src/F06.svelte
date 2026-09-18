<script>
	export let x = 0;
	$: a = ((x) => x)(1);
	$: x = a;
</script>
