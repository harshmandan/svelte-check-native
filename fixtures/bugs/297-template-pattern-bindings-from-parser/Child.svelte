<script lang="ts">
    import type { Snippet } from 'svelte';
    let { row }: { row?: Snippet<[Map<() => void, string>]> } = $props();
</script>

{@render row?.(new Map())}
