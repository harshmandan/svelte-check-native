<svelte:options runes />
<script lang="ts">
    // Reviewer item #3b: aliased import + multiple dispatchers.
    // Pre-fix our `find_dispatcher_event_type_source` matched only
    // the simple-identifier callee `createEventDispatcher`, so:
    //   - `import { createEventDispatcher as ced }`'s `ced<T>()`
    //     got no narrowing.
    //   - `const a = ced<A>(), b = ced<B>()` only the first hit
    //     was used.
    //
    // The new `collect_ctor_locals` resolves the alias chain so
    // `ced` is recognised as a dispatcher constructor; the same
    // helper threads through `find_dispatcher_event_type_source`
    // and `find_dispatcher_local_names`.
    import { createEventDispatcher as ced } from 'svelte'

    let { label = '' }: { label?: string } = $props()

    const dispatch = ced<{ click: { id: number } }>()

    function fire() {
        dispatch('click', { id: 1 })
    }
</script>

<button onclick={fire}>{label}</button>
