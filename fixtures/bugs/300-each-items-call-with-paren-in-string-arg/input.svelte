<script lang="ts">
    const f = (s: string) => (t: string) => [s.length, t.length];
</script>

{#each f("(")(")") as row}<slot name="x" r={row} />{/each}
