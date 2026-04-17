<script lang="ts">
    // +page.svelte with an untyped $props() destructure. Our emit
    // recognizes the route-file basename and injects
    // `data: import('./$types.js').PageData` (and for `form`,
    // `import('./$types.js').ActionData`) into the synthesized
    // default-export Props shape. Downstream overlays reading
    // `ComponentProps<typeof Page>` then resolve to the real shape
    // instead of `any`.
    let { data, form } = $props();
</script>

<h1>{data.title}</h1>
<p>count = {data.count}</p>
{#if form}
    <span>form action submitted</span>
{/if}
