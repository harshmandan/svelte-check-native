<script lang="ts">
	let el: HTMLDivElement | null = $state(null); const bad: string = 123;
	void el;
	void bad;
</script>

<div bind:this={el}></div>
