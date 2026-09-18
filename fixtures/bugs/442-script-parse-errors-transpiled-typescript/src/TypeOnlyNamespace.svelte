<script lang="ts">
	function N() {}
	namespace N {
		export type X = 1;
	}
	const x: N.X = 1;
	console.log(N, x);
</script>
