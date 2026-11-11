<script>
    /** Narrow, see {@link Props @type} */
    export let linked = 'a';
    /** @see Props@type {'a' | 'b'} */
    export let glued = 'a';
    /** Mid-line x @type {'a' | 'b'} */
    export let midline = 'a';
    /** @see `@type {'a' | 'b'}` */
    export let ticked = 'a';
</script>

{#if linked === 'b'}b{/if}
{#if glued === 'b'}b{/if}
{#if midline === 'b'}b{/if}
{#if ticked === 'b'}b{/if}
