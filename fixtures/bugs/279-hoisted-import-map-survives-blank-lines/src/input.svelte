<script lang="ts">
	import { onMount } from "svelte"

	import type { Missing } from "not-a-real-pkg"

	const x: Missing | null = null
	onMount(() => {})
</script>

<p>{x}</p>
