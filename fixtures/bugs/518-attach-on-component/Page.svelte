<script lang="ts">
	import Box from './Box.svelte'
</script>

<Box
	{@attach (element) => {
		element.scrollTop = 0
	}}
/>
<Box {@attach (element) => element.notAProperty} />
