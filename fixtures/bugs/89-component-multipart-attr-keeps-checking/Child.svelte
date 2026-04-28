<svelte:options runes />
<script lang="ts">
    let { label, count }: { label: string; count: number } = $props()
</script>

<span class="lbl">{label} ({count})</span>
