<svelte:options runes />
<script lang="ts">
    // Typed-dispatcher case for the post-#2 event-surface contract.
    // The dispatcher's `T` is the DETAIL map (`dispatch('click', detail)`
    // emits `CustomEvent<detail>`); emit wraps it ONCE at synthesis to
    // produce the FINAL `$on`-typed event map. Render-fn return,
    // `__svn_events` marker, and `__svn_ensure_component` all treat
    // `$$Events` as the final $on map and never wrap again — so
    // every consumer path lands at exactly one wrap level.
    //
    // Runes mode is the trigger that pulls the dispatcher's type
    // arg into the synthesized event surface (legacy
    // `createEventDispatcher` is still permitted in runes mode).
    // Without one of the three triggers (an explicit Events
    // interface, the strictEvents script attribute, or runes mode),
    // the dispatcher's typing stays lax — see `narrow_events` in
    // emit/lib.rs.
    import { createEventDispatcher } from 'svelte'

    let { label = '' }: { label?: string } = $props()

    const dispatch = createEventDispatcher<{ click: { id: number } }>()

    function fire() {
        dispatch('click', { id: 1 })
    }
</script>

<button onclick={fire}>{label}</button>
