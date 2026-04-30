<script lang="ts">
    import Inner from './Inner.svelte'

    // After the duplicate-collapse fix, `hi` is `CustomEvent<any>` —
    // a bare-CustomEvent handler accepts it. Without the computed-key
    // resolution, `hi` would have been `CustomEvent<boolean & string>`
    // = `CustomEvent<never>`, and TS would reject every handler.
    function handleAny(e: CustomEvent): void {
        void e.detail
    }
</script>

<Inner on:hi={handleAny} />
