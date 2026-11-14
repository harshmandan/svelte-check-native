<script lang="ts">
	const level = 2
	const label = 'x'
</script>

<svelte:head>
	<style media={level}>p { color: red; }</style>
</svelte:head>
<div>
	<style title={label}>.a { color: blue; }</style>
	<p class="a">styled</p>
</div>
