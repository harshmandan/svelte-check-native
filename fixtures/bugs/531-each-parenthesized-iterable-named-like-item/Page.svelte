<script lang="ts">
    let items = [1, 2];
    let maybe: number[] | null = [1];
</script>

{#each (items) as items}{items}{/each}
{#each (maybe!) as maybe}{maybe}{/each}
{#each ((items)) as items}{items}{/each}
