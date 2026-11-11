<script lang="ts">
	let { title }: { title?: string; [key: symbol]: (node: HTMLDivElement) => void } = $props()
</script>

<div>{title}</div>
