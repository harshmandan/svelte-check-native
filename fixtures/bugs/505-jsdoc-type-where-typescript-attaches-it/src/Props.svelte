<script>
    export /** @type {'a' | 'b'} */ let between = 'a';
    /** @type {'a' | 'b'} */
    export let before = 'a';
    export let first = 'a', /** @type {'a' | 'b'} */ own = 'a';
</script>

{#if between === 'b'}b{/if}
{#if before === 'b'}b{/if}
{#if first === 'b'}b{/if}
{#if own === 'b'}b{/if}
