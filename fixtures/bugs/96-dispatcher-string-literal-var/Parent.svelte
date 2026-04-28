<svelte:options runes />
<script lang="ts">
    import Child from './Child.svelte'

    function wrong(e: number) {
        void e
    }
</script>

<!-- The dispatched event name `click` came from the const-bound
     `EV = 'click'` in Child. Synth produces $$Events = { click:
     CustomEvent<any> }. Consumer's `(e: number)` handler doesn't
     match `(e: CustomEvent<any>)` → TS2345. -->
<Child on:click={wrong} />
