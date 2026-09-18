<script lang="ts">
	function pick(v: string): string;
	function pick(v: number): number;
	function pick(v: unknown) {
		return v;
	}
	const data = await Promise.resolve(pick("a"));
	console.log(data);
</script>
