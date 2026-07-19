<script lang="ts">
	let el: HTMLInputElement | null = $state(null);
	if (el) el.focus();
</script>

<input bind:this={el} />
