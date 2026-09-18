<script>
	let { p } = $props();
</script>
{#await p then v}{@const v = 1}{v}{/await}
