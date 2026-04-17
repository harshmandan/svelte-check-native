<script lang="ts">
    import Wrapper from './Wrapper.svelte';
    import Inner from './Inner.svelte';
</script>

<Wrapper>
    {#snippet children(isOpen, toggle)}
        <Inner open={isOpen} onToggle={toggle} />
    {/snippet}
</Wrapper>
