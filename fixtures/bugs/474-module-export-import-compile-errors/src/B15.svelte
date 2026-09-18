<script>
	import Comp from "./B16.svelte";
</script>
<Comp bind:title={x}>{#snippet title()}x{/snippet}</Comp>
