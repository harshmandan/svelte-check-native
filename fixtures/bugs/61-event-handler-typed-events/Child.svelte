<script lang="ts">
    // Svelte-4-style component with a strict `$$Events` declaration.
    // Consumers writing `<Child on:myevent={h}>` must have their
    // handler `h` type-checked against `CustomEvent<{ id: number }>`.
    //
    // No `createEventDispatcher` call here — svelte's own
    // `EventMap extends Record<string, unknown>` constraint on
    // dispatcher is orthogonal to the consumer-side typing path
    // this fixture locks. `$$Events` alone is what emit looks at.
    interface $$Events {
        myevent: { id: number }
    }

    export let label: string = ''
</script>

<button>{label}</button>
