<script lang="ts">
    import * as Ns from './NsGone.svelte';
    let show = true;
</script>

<Ns.Comp a={1} />
<p>{Ns.value}</p>
{#await import('./AwaitGone.svelte') then m}<m.default />{/await}
<div data-x={import('./AttrGone.svelte')}></div>
{#snippet s(p: import('./SnippetGone.svelte').Props)}<p>{p}</p>{/snippet}
{#if show}
    {@const L = import('./ConstGone.svelte')}
    <p>{L}</p>
{/if}
{@render s(1)}
