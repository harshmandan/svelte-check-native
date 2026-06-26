<script module>
    // module script present but does not reference the snippet
</script>

<script lang="ts">
    import type { Component } from 'svelte';
    const { icon: Icon }: { icon: Component<{ size: number }> } = $props();
</script>

{#snippet iconSnippet()}
    <Icon size={16} />
{/snippet}

{@render iconSnippet()}
