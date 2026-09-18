<script>
	import Comp from "./B16.svelte";
</script>
<Comp {title}>{#snippet title()}x{/snippet}</Comp>
<svelte:self title="a">{#snippet title()}x{/snippet}</svelte:self>
