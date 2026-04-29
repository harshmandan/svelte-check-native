<script lang="ts">
    import Producer from './Producer.svelte'

    // The `(typeof Producer)[]` pattern requires the callable-only
    // `Component<P, X, B>` shape (the iso interface's `new(...)`
    // ctor signature can't be satisfied by a callable-only function
    // expression). Pre-round-7-#7, Producer's `<svelte:self on:click>`
    // counted as a bubbled component event, pushing it onto the iso
    // shape — so this assignment fired TS2322. Post-fix, Producer is
    // free of bubbled events and lands on the fn-component shape.
    function asArray(): (typeof Producer)[] {
        return [Producer]
    }
    void asArray
</script>
