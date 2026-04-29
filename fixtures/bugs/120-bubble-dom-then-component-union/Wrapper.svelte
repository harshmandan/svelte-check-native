<svelte:options runes />
<script lang="ts">
    // Round-6 follow-up #2: DOM-then-component bubble merge must
    // UNION, not REPLACE. Pre-fix native's merge had a special case
    // where a Component source arriving on top of a DOM entry
    // REPLACED the entry with just the component projection — a
    // consumer's MouseEvent handler then silently failed to type-
    // check (the union projection was thrown away).
    //
    // Upstream `event-handler.ts:55-60` handles a component bubble
    // by `bubbledEvents.set(name, [].concat(exist, exp))`, which
    // APPENDS to whatever is already in the map (DOM or component).
    // Same name DOM-then-component therefore produces
    // `unionType(DOM_expr, comp_expr)` at emit time.
    //
    // This fixture has the DOM bubble FIRST (`<button on:click>`)
    // followed by the component bubble (`<Inner on:click />`). The
    // wrapper's `$$Events.click` should be the union
    // `MouseEvent | CustomEvent<{ id: number }>` — both handler
    // shapes a consumer might write must type-check.
    import Inner from './Inner.svelte'
    let { label = '' }: { label?: string } = $props()
    void label
</script>

<button on:click>{label}</button>
<Inner on:click />
