<script lang="ts">
	import Parent from './Parent.svelte'
	import Badge from './Badge.svelte'
	const name = 'badge'
</script>

<Parent><Badge slot="{name}" /></Parent>
