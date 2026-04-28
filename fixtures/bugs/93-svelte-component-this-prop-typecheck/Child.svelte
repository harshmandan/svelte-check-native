<svelte:options runes />
<script lang="ts">
    let { value }: { value: number } = $props()
</script>

<span>{value}</span>
