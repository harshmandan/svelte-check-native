<svelte:options runes />
<script lang="ts">
    // Strict Props — anything that isn't `label` must be excess.
    let { label = '' }: { label?: string } = $props()
    void label
</script>

<button>{label}</button>
