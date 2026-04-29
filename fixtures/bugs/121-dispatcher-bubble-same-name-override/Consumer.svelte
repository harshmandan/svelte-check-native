<script lang="ts">
    import Inner from './Inner.svelte'

    // `click` is overridden by the DOM bubble — its type is
    // `MouseEvent`, NOT the dispatcher's
    // `CustomEvent<{ id: number }>`. A MouseEvent handler passes.
    function handleMouseClick(e: MouseEvent): void {
        void e.clientX
    }

    // `dispatched` keeps the dispatcher's projection (no bubble
    // override) — it's `CustomEvent<string>`. A handler typed for
    // that shape passes.
    function handleDispatched(e: CustomEvent<string>): void {
        void e.detail
    }

    // A handler typed against the dispatcher's `click` payload is
    // now WRONG — the bubble overrode the dispatcher entry, so
    // `click` is `MouseEvent` not `CustomEvent<{ id: number }>`.
    // Must fire TS2345.
    function handleStaleCustom(e: CustomEvent<{ id: number }>): void {
        void e.detail.id
    }
</script>

<Inner on:click={handleMouseClick} on:dispatched={handleDispatched} />
<Inner on:click={handleStaleCustom} />
