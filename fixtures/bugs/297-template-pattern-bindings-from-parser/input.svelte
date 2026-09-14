<script lang="ts">
    const k = 'a';
    const p: Promise<number> = Promise.resolve(1);
</script>

{#await p}{:then n}{n}{:catch { [k]: v }}{v}{/await}
