<script lang="ts">
    // Svelte-4-style component with a strict `$$Events` declaration.
    // Consumers writing `<Child on:myevent={h}>` must have their
    // handler `h` type-checked against `CustomEvent<{ id: number }>`.
    //
    // Convention (post-#2 event-surface refactor, mirrors upstream
    // svelte2tsx): `$$Events` is the FINAL `$on` event-object map.
    // Users write `CustomEvent<…>` explicitly when they want it.
    // Pre-refactor we wrapped at multiple sites in the shim chain,
    // letting users omit the wrap; the redundant wraps risked
    // double-wrapping and diverged from upstream.
    //
    // No `createEventDispatcher` call here — svelte's own
    // `EventMap extends Record<string, unknown>` constraint on
    // dispatcher is orthogonal to the consumer-side typing path
    // this fixture locks. `$$Events` alone is what emit looks at.
    interface $$Events {
        myevent: CustomEvent<{ id: number }>
    }

    export let label: string = ''
</script>

<button>{label}</button>
