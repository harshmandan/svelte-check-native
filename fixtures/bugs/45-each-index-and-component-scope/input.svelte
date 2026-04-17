<script lang="ts">
    import Inner from './Inner.svelte';
    const items: { text: string }[] = [{ text: 'a' }, { text: 'b' }];
</script>

{#each items as item, i}
    <Inner label={item.text} index={i} />
{/each}
