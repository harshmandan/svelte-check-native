<script lang="ts">
    import Child from './Child.svelte'

    // Wrong payload shape: declared `{id: number}`, parent expects
    // `{name: string}`. Must fire TS2345 on the handler argument
    // at the `on:myevent={wrongHandler}` directive position.
    function wrongHandler(e: CustomEvent<{ name: string }>): void {
        void e.detail.name
    }
</script>

<Child on:myevent={wrongHandler} />
