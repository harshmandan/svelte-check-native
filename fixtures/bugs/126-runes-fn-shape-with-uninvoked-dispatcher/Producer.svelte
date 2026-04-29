<script lang="ts">
    // Round-7 follow-up #6: this Producer creates an UNTYPED
    // `createEventDispatcher()` and never calls `dispatch('name', …)`
    // with a string literal — so upstream's `events.size` stays 0
    // and `events.hasEvents()` is false. Pre-fix native's gate used
    // `has_dispatcher_call` (true the moment ANY
    // `createEventDispatcher()` exists), so this Producer was pushed
    // onto the iso `$$IsomorphicComponent` shape and the consumer's
    // `(typeof Producer)[]` pattern fired TS2322. Post-fix native's
    // gate uses `has_concrete_dispatcher_events` (= synthesised
    // events-type is some) which only fires when the dispatcher
    // contributed concrete event names — none here.
    import { createEventDispatcher } from 'svelte'
    let { id }: { id: string } = $props()
    void id

    const _dispatch = createEventDispatcher()
    void _dispatch
</script>

<p>id={id}</p>
