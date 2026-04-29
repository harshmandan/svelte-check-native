<script lang="ts">
    // Round-8 follow-up #4: this Producer creates a TYPE-REFERENCE
    // typed dispatcher (`createEventDispatcher<MyEvents>()`) and
    // never invokes it. Upstream's `events.size > 0` check
    // (ComponentEvents.ts:231) only fires for INLINE type literals
    // because `dispatcherTyping.members` is only enumerable on
    // `ts.TypeLiteral` — a `ts.TypeReference` to an alias has no
    // member-list to walk, so events.size stays 0 and
    // events.hasEvents() is false → upstream picks the fn-component
    // shape. Pre-fix native gated on `synthesized_events_type
    // .is_some()` which fired for ANY typed dispatcher (literal or
    // ref) and pushed this Producer onto the iso shape, breaking
    // `(typeof Producer)[]` consumer patterns. Post-fix native uses
    // `has_inline_typed_dispatcher_members || synthesized_untyped
    // _events.is_some()` so type-ref-only typed dispatchers stay
    // on the fn shape.
    import { createEventDispatcher } from 'svelte'
    type MyEvents = { foo: string; bar: number }
    let { id }: { id: string } = $props()
    void id

    const _dispatch = createEventDispatcher<MyEvents>()
    void _dispatch
</script>

<p>id={id}</p>
