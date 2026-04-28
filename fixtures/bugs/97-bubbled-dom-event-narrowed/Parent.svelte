<svelte:options runes />
<script lang="ts">
    import Child from './Child.svelte'

    // Correct shape: the bare `<button on:click>` in Child bubbles a
    // native DOM `click` event, so the consumer's handler should
    // accept `MouseEvent`. Matches `$on<'click'>(handler: (e:
    // HTMLElementEventMap['click']) => any)` exactly.
    function correctHandler(e: MouseEvent): void {
        void e.clientX
    }

    // WRONG shape: declared `(e: number)` doesn't accept `MouseEvent`.
    // Must fire TS2345 at the directive's value position. Pre-fix this
    // was silently passing because the consumer side defaulted to
    // `(e: CustomEvent<any>)`.
    function wrongHandler(e: number): void {
        void e
    }
</script>

<Child on:click={correctHandler} />
<Child on:click={wrongHandler} />
