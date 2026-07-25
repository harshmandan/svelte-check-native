<script lang="ts">
    import type { PageProps } from './$types';

    let { data, form }: PageProps = $props();

    const needsNumber = (n: number) => n;
</script>

<h1>{data.title}</h1>
<p>{needsNumber(data.count)}</p>
<p>{needsNumber(data.title)}</p>
{#if form?.ok}
    <span>submitted</span>
{/if}
