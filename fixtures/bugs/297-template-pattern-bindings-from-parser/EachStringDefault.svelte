<script lang="ts">
    const xs: { a?: string }[] = [];
    const qq = 1;
</script>

{#each xs as { a = 'p,qq' }}{a}{/each}
