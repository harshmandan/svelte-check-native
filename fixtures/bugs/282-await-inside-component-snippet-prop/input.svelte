<script lang="ts">
    import Wrapper from './Wrapper.svelte';
    import Row from './Row.svelte';

    let page: Promise<string> = Promise.resolve('x');
</script>

<Wrapper>
    {#snippet children()}
        {#await page then text}
            <p>{text.length}</p>
        {:catch err}
            <p>{err}</p>
        {/await}
    {/snippet}
</Wrapper>

<Row>
    {#snippet row(label)}
        {#await page then text}
            <p>{label.toUpperCase()}{text}</p>
        {/await}
    {/snippet}
</Row>
