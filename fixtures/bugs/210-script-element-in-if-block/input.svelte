<script lang="ts">
	let load = true;
	let loadNonce = true;

	function handleLoad() {
		load = false;
	}

	function handleNonceLoad() {
		loadNonce = false;
	}
</script>

{#if load}
	<script defer src="https://example.com/some-script.js" onload={handleLoad}></script>
{/if}

{#if loadNonce}
	<script
		defer
		nonce="%sveltekit.nonce%"
		src={`https://example.com/lib@${'1.0.0'}/script.js`}
		onload={handleNonceLoad}
	></script>
{/if}
