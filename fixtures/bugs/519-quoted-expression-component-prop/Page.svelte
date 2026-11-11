<script lang="ts">
	import Counter from './Counter.svelte'

	const total = 3
	const label = 'three'
</script>

<Counter count="{total}" />
<Counter count='{total}' />
<Counter count="{label}" />
