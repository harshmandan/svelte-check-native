<script lang="ts">
    import Inner from './Inner.svelte'

    // Post-fix: `save`'s detail is `string` (from the second typed
    // dispatcher; the first uses an unsupported computed
    // string-literal key that's silently dropped from the
    // duplicate-collapse pass). A handler typed
    // `(e: CustomEvent<number>) => void` is NOT assignable —
    // TS2322 on `on:save`. Pre-fix the phantom 'save' from the
    // computed form duplicated against the second source and the
    // event collapsed to `CustomEvent<any>`, accepting any handler.
    function handleNum(e: CustomEvent<number>): void {
        void e.detail
    }
</script>

<Inner on:save={handleNum} />
