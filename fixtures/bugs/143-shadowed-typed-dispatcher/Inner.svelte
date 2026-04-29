<script lang="ts">
    // Round-11 follow-up #3: a top-level UNTYPED dispatcher is
    // shadowed by a nested TYPED dispatcher with the same name.
    // Pre-fix native subtracted typed_locals from all_locals by
    // NAME — both entries had name "dispatch" so the multiset
    // difference excluded all of them, and the top-level
    // `dispatch('bar', …)` call was never collected. The events
    // surface dropped 'bar' silently.
    //
    // Upstream's per-call check (`ComponentEvents.ts:256`) tests
    // `eventDispatchers.some(d => !d.typing && d.name === call.name)`
    // — at least ONE untyped dispatcher with the matching name.
    // The shadow case has one (the top-level binding), so calls
    // collect.
    import { createEventDispatcher } from 'svelte'
    const dispatch = createEventDispatcher()
    dispatch('bar')

    function nested() {
        const dispatch = createEventDispatcher<{ scoped: number }>()
        void dispatch
    }
    void nested
</script>

<button>x</button>
