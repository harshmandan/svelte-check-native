<script lang="ts">
    import type { Snippet } from 'svelte';
    let { row }: { row: Snippet<[label: string]> } = $props();
</script>

{@render row('a')}
