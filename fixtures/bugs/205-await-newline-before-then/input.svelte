<script lang="ts">
    const p: Promise<number> = Promise.resolve(1);
    const q: Promise<string> = Promise.reject(new Error('nope'));

    function fetchData(url: string, retries: number): Promise<boolean> {
        void url;
        void retries;
        return Promise.resolve(true);
    }
</script>

{#await p
    then v}
    <span>{v.toFixed(2)}</span>
{/await}

{#await q	catch e}
    <span>{String(e)}</span>
{/await}

{#await fetchData(
    'https://example.com',
    3
)
    then ok}
    <span>{ok ? 'yes' : 'no'}</span>
{/await}
