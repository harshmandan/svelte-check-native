<script lang="ts">
    function label(n: string): string { return n; }
</script>

{#each Array.from({ length: 3 }), i}
    <p>{label(i)}</p>
{/each}
