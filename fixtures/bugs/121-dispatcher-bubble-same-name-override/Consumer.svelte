<script lang="ts">
    import Inner from './Inner.svelte'

    // `click` has BOTH a typed dispatcher entry AND a DOM bubble.
    // Per upstream `ComponentEvents.ts:283-286, 304`, the same-name
    // collision widens the effective type to `CustomEvent<any>` (the
    // duplicate-key-last-wins `'click': __sveltets_2_customEvent`
    // entry). A `MouseEvent` handler does NOT match — `CustomEvent<any>`
    // is structurally different from `MouseEvent` (DOM event vs
    // CustomEvent shape). Must fire TS2345.
    function handleMouseClick(e: MouseEvent): void {
        void e.clientX
    }

    // `dispatched` keeps the dispatcher's projection (no bubble
    // override) — it's `CustomEvent<string>`. A handler typed for
    // that shape passes.
    function handleDispatched(e: CustomEvent<string>): void {
        void e.detail
    }

    // A handler typed against the dispatcher's `click` payload
    // PASSES bivariantly — `CustomEvent<{id}>` and `CustomEvent<any>`
    // are mutually assignable through `any`'s bidirectional
    // assignability.
    function handleStaleCustom(e: CustomEvent<{ id: number }>): void {
        void e.detail.id
    }
</script>

<Inner on:click={handleMouseClick} on:dispatched={handleDispatched} />
<Inner on:click={handleStaleCustom} />
