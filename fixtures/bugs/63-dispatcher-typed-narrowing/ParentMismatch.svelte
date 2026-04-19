<script lang="ts">
    import Child from './Child.svelte'

    // Wrong payload shape: Child declares via dispatcher generic
    // `myevent: { id: number }`; this handler expects `{ name:
    // string }`. TS2345 must fire at the on:myevent={wrongHandler}
    // directive position.
    function wrongHandler(e: CustomEvent<{ name: string }>): void {
        void e.detail.name
    }
</script>

<Child on:myevent={wrongHandler} />
