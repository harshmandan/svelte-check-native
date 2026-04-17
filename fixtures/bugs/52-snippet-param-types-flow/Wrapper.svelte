<script lang="ts">
    import type { Snippet } from 'svelte';
    let { row, header }: {
        row: Snippet<[{ id: number; label: string }]>;
        header: Snippet<[columns: readonly string[]]>;
    } = $props();
</script>

{@render header(['id', 'label'])}
{@render row({ id: 1, label: 'a' })}
