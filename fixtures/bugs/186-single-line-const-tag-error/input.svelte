<script lang="ts">
    let items: number[] = [1, 2, 3];
</script>

{#each items as item}
    {@const doubled = nonexistentRef + item}
    <p>{doubled}</p>
{/each}
