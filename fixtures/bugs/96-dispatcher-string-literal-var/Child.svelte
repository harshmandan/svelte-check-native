<svelte:options runes />
<script lang="ts">
    // Reviewer item #3c (string-literal event variables): a
    // dispatched event whose name is a `const X = 'click'`-bound
    // variable should appear in the synthesised event surface.
    // Pre-fix only string-literal first args were collected, so
    // `dispatch(EV, payload)` (where `EV` was a const-bound
    // string) contributed nothing and the consumer's `on:click`
    // got no narrowing.
    //
    // The new pre-pass collects `const NAME = 'literal'` bindings
    // and resolves identifier args through that map.
    import { createEventDispatcher } from 'svelte'

    const dispatch = createEventDispatcher()
    const EV = 'click'

    let { label = '' }: { label?: string } = $props()

    function fire() {
        dispatch(EV, { id: 1 })
    }
</script>

<button onclick={fire}>{label}</button>
