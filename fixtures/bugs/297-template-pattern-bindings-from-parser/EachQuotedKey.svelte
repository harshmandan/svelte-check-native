<script lang="ts">
    const xs: { 'my-key': number }[] = [];
    const key = 1;
</script>

{#each xs as { 'my-key': v }}{v}{/each}
