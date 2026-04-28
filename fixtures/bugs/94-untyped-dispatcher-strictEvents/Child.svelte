<svelte:options runes />
<script lang="ts">
    // Reviewer item #3a: untyped `createEventDispatcher()` under
    // runes mode (one of the three event-narrowing triggers).
    // Pre-fix our synth path only handled typed
    // `createEventDispatcher<T>()` and skipped untyped dispatchers
    // entirely — consumers' `on:click` / `on:foo` got `(e: any)`
    // with no narrowing to the actual dispatched-name set.
    //
    // The new slice scans for `dispatch('name', …)` calls and
    // synthesises a detail map (`{ click: any, foo: any }`).
    // The wrap-once at synthesis (item #2) then produces the
    // FINAL `$$Events = { click: CustomEvent<any>, foo:
    // CustomEvent<any> }`.
    import { createEventDispatcher } from 'svelte'

    const dispatch = createEventDispatcher()

    let { label = '' }: { label?: string } = $props()

    function fire() {
        dispatch('click', { id: 1 })
        dispatch('foo', 'detail')
    }
</script>

<button onclick={fire}>{label}</button>
