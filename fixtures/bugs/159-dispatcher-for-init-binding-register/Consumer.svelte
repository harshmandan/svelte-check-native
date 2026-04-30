<script lang="ts">
    import Inner from './Inner.svelte'

    // Post-fix `save` is `CustomEvent<any>` (overridden by the
    // dispatched-name layer). A handler typed as
    // `(e: CustomEvent<string>) => void` is assignable. Pre-fix the
    // for-init dispatcher binding was missed by
    // `scan_statement_in_source_order`, the dispatched 'save'
    // never reached the surface, and the typed source kept
    // `save: CustomEvent<number>` — TS2345 on every non-`number`
    // handler.
    function handleSave(e: CustomEvent<string>): void {
        void e.detail
    }
</script>

<Inner on:save={handleSave} />
