<script lang="ts">
    // Round-6 follow-up #1: when the SAME event name is BOTH
    // dispatched (typed `createEventDispatcher<T>`) AND bubbled from
    // a DOM element (`<button on:click>`), upstream's emit treats
    // the bubble as a JS object literal SPREAD AFTER the dispatcher
    // mapped type — bubble keys OVERRIDE dispatcher keys (compare
    // upstream's `ts-event-dispatchers-same-event/expectedv2.ts`:
    // `events: {...toEventTypings<{...}>(), 'click': mapElementEvent('click'), …}`).
    //
    // Pre-fix native intersected the dispatcher and bubble fragments
    // with `&` — same-name collisions collapsed to
    // `CustomEvent<{detail}> & MouseEvent`, neither shape a usable
    // consumer-handler-arg type. Post-fix native folds the bubble
    // map into the dispatcher mapped type with TS-level spread
    // semantics: `Omit<Dispatcher, keyof Bubble> & Bubble` so the
    // bubble's `MouseEvent` projection wins for `click` while the
    // dispatcher's `CustomEvent<{ id: number }>` projection still
    // covers the unique `dispatched` name.
    import { createEventDispatcher } from 'svelte'
    const dispatch = createEventDispatcher<{
        click: { id: number }
        dispatched: string
    }>()
    void dispatch
</script>

<button on:click>inner</button>
