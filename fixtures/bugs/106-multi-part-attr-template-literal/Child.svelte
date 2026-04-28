<svelte:options runes />
<script lang="ts">
    let { label }: { label: string } = $props()
    void label
</script>

<button class={label}>{label}</button>
