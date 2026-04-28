<script lang="ts">
    // Reviewer follow-up #1: bare `<Inner on:NAME />` (no value) is
    // event-bubble shorthand — Wrapper re-dispatches Inner's NAME
    // event to its own consumers. Pre-fix the walker only flagged
    // `has_bubbled_component_event` and skipped the `$on` call
    // entirely, so a bubbled event name the child doesn't declare
    // passed silently. Post-fix the walker pushes an
    // `OnEventDirective` with an empty handler_range and emit
    // produces `$inst.$on("NAME", () => {})` — TS narrows the name
    // against Inner's declared `$$Events` and unknown names fire
    // TS2769 ('No overload matches this call') / TS2345.
    //
    // `<Inner on:click />` is a valid bubble (click is in Inner's
    // $$Events). `<Inner on:not_a_real_event />` must fire because
    // not_a_real_event isn't a declared key.
    import Inner from './Inner.svelte'
</script>

<Inner on:click />
<Inner on:not_a_real_event />
