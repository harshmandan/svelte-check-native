<script lang="ts">
    import Inner from './Inner.svelte'

    // Post-fix `save` is `CustomEvent<any>` (overridden by the
    // dispatched-name layer because the let-aliased `EV` resolves
    // to 'save'). A handler typed `(e: CustomEvent<string>) => void`
    // is assignable. Pre-fix the let alias was dropped, the typed
    // source's `save: CustomEvent<number>` survived, and the
    // wrong-typed handler fired TS2345.
    function handleSave(e: CustomEvent<string>): void {
        void e.detail
    }
</script>

<Inner on:save={handleSave} />
