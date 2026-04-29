<svelte:options runes />
<script lang="ts">
    // Reviewer follow-up #2: bare `<Inner on:click />` re-dispatches
    // Inner's typed `click` event up to Wrapper's own consumers. The
    // Wrapper's `$$Events` surface must carry `click` with Inner's
    // declared event type.
    //
    // Pre-fix the walker only flagged `has_bubbled_component_event`
    // and recorded the local `$on(...)` for child-event-name
    // validation; the wrapper's `events_alias_body` was built only
    // from dispatcher synth + DOM bubbles, so the propagated bubble
    // never showed up. Post-fix the walker also records
    // `bubbled_component_events: [(click, Inner)]` and emit projects
    // `__SvnComponentEvents<typeof Inner>["click"]` into
    // `events_alias_body`, intersected with the rest.
    import Inner from './Inner.svelte'
    // `$props()` call is what flips `is_runes_mode` on (the
    // detection scans the script for rune CALLs, not the
    // `<svelte:options runes />` attribute).
    let { label = '' }: { label?: string } = $props()
    void label
</script>

<Inner on:click />
