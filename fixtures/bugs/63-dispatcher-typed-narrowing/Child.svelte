<script lang="ts">
    // Child uses `createEventDispatcher<T>()` without an `interface
    // $$Events`. v0.3 Item 5 extracts T and synthesizes `type
    // $$Events = T` at the overlay's module scope, so Item 3's
    // `__svn_events` intersection fires the same way as for
    // declared interfaces.
    import { createEventDispatcher } from 'svelte'

    type DispatcherEvents = {
        myevent: { id: number }
    }

    const dispatch = createEventDispatcher<DispatcherEvents>()

    export let label: string = ''

    function go() {
        dispatch('myevent', { id: 1 })
    }

    void go
    void dispatch
</script>

<button>{label}</button>
