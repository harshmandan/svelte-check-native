<script lang="ts">
    let xs = $state([{ a: 1, b: 'x' }]);
</script>

{#each xs as { a, ...rést }}<input bind:value={rést.b} />{a}{/each}
{#each xs as { a, ...rest }}<input bind:value={/* c */ rest.b} />{a}{/each}
