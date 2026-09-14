<script lang="ts">
    const Other = 1;
    type Other = string;
</script>

{#snippet s(a: Map<() => void, Other>)}{a.size}{/snippet}
{@render s(new Map())}
