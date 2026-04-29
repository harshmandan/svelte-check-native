<script lang="ts">
    // Round-8 follow-up #5: TWO inline typed dispatchers declare the
    // same event name `hi` with INCOMPATIBLE detail types
    // (`boolean` vs `string`). Upstream's `addToEvents`
    // (ComponentEvents.ts:279) detects the second `addToEvents('hi',
    // …)` collision and pushes 'hi' into `dispatchedEvents`, which
    // `toDefString` then emits last as `'hi': customEvent` —
    // overriding both spreads with `CustomEvent<any>`. See
    // `language-tools/test/svelte2tsx/samples/ts-event-dispatchers-
    // same-event/expectedv2.ts`.
    //
    // Pre-fix native intersected the typed sources at the type
    // level: `({hi: boolean}) & ({hi: string})` gave `hi: boolean &
    // string` = `never`, which TS rejected on every consumer
    // handler. Post-fix native detects the inline-literal duplicate
    // and pushes 'hi' into the untyped-names layer (which round-7
    // #5's order already overrides last with `CustomEvent<any>`),
    // matching upstream.
    import { createEventDispatcher } from 'svelte'
    const _d1 = createEventDispatcher<{ hi: boolean }>()
    const _d2 = createEventDispatcher<{ hi: string }>()
    void _d1
    void _d2
</script>

<button>x</button>
