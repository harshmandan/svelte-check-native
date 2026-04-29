<script lang="ts">
    import Wrapper from './Wrapper.svelte'

    // Wrapper's `click` is the union of MouseEvent (DOM) and
    // Inner's CustomEvent<{ id: number }>. Both handler shapes
    // must type-check — a handler that accepts the union (or one
    // that accepts the broader of the two) passes.
    function handleEither(
        e: MouseEvent | CustomEvent<{ id: number }>
    ): void {
        void e
    }

    // A handler typed only for MouseEvent is NOT assignable to
    // the union — TS rejects it because the union is wider than
    // MouseEvent. Must fire TS2345.
    function handleMouseOnly(e: MouseEvent): void {
        void e.clientX
    }
</script>

<Wrapper on:click={handleEither} />
<Wrapper on:click={handleMouseOnly} />
