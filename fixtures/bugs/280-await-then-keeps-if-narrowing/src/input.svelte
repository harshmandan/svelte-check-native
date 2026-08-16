<script lang="ts">
    let box: { page: Promise<string>; version: number } | undefined = undefined;
    if (Math.random() > 0.5) {
        box = { page: Promise.resolve("x"), version: 1 };
    }
</script>

{#if box}
    {#await box.page}
        <p>{box.version}</p>
    {:then text}
        <p>{text.length} {box.version}</p>
    {:catch err}
        <p>{err}</p>
    {/await}
{/if}

{#snippet inner()}
    {#if box}
        {#await box.page then text}
            <p>{text} {box.version}</p>
        {/await}
    {/if}
{/snippet}
{@render inner()}
