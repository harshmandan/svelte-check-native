<script lang="ts">
    // Round-9 follow-up #2: TWO typed dispatchers with TYPE-ALIAS
    // type args sharing an event name `hi`. Round-8 #5's inline-
    // literal duplicate-collapse can't see the alias members at
    // synth time, so the duplicate detection misses this case.
    //
    // Pre-fix native intersected the typed sources at the type
    // level — `(EventsA) & (EventsB)` made `hi`'s detail
    // `boolean & string` = `never`, so consumer handlers failed.
    //
    // Upstream emits `...toEventTypings<EventsA>(),
    // ...toEventTypings<EventsB>()` and JS spread last-wins makes
    // `hi` resolve to `EventsB`'s typing. Native now mirrors this
    // at the type level via `Omit<(EventsA), keyof (EventsB)> &
    // (EventsB)` — same source-order-last-wins semantic without
    // needing key enumeration.
    import { createEventDispatcher } from 'svelte'
    type EventsA = { hi: boolean; foo: number }
    type EventsB = { hi: string; bar: boolean }
    const _d1 = createEventDispatcher<EventsA>()
    const _d2 = createEventDispatcher<EventsB>()
    void _d1
    void _d2
</script>

<button>x</button>
