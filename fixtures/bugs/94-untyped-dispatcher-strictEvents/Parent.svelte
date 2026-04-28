<svelte:options runes />
<script lang="ts">
    import Child from './Child.svelte'

    // Correct shape — `(e: CustomEvent<any>) => void` matches the
    // synthesised `$$Events.click = CustomEvent<any>` entry.
    function handleClick(e: CustomEvent) {
        void e.detail
    }

    // WRONG shape — `(e: number)` doesn't accept `CustomEvent<any>`.
    // Must fire TS2345 at the directive's value position.
    function wrongHandler(e: number) {
        void e
    }
</script>

<Child on:click={handleClick} on:foo={wrongHandler} />
