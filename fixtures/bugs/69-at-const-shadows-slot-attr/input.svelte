<script lang="ts">
    let item = { id: 1 };
</script>

{#each [item] as one}
    {@const expanded = one.id}
    <slot {expanded} />
{/each}
