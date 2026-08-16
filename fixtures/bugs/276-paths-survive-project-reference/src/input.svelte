<script lang="ts">
    // Resolves through the entry tsconfig's $lib paths mapping. The
    // sibling composite reference (same directory, no paths of its
    // own) must not shadow that mapping.
    import Thing from "$lib/thing.svelte";
</script>
<Thing n={1} />
