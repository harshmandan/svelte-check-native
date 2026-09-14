<script lang="ts">
    const zed = 1;
</script>

{#snippet s(a: 'x' | 'y,zed')}{a}{/snippet}
{@render s('x')}
