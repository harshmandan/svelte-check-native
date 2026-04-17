<script lang="ts">
    import type { Snippet } from 'svelte';
    let { children }: { children: Snippet<[isOpen: boolean, toggle: () => void]> } = $props();
</script>

{@render children(true, () => {})}
