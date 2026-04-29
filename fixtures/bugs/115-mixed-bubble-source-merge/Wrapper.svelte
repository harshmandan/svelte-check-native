<svelte:options runes />
<script lang="ts">
    // Reviewer follow-up #1 (round 5): when a wrapper has BOTH a
    // `<Child on:click />` (component bubble) AND a `<button on:click>`
    // (DOM bubble) for the SAME event name, pre-fix the two were
    // built into separate type fragments and INTERSECTED:
    //   `(__SvnComponentEvents<typeof Child>['click']) & MouseEvent`
    // — neither shape is a usual handler argument and TS rejected most
    // user handlers.
    //
    // Upstream svelte2tsx runs all bubbles through one
    // `EventHandler.bubbledEvents` map. DOM bubbles call
    // `Map.set(name, expr)` which OVERWRITES the entire entry;
    // component bubbles call `set(name, [].concat(exist, exp))`
    // which APPENDS to whatever is already there (DOM or component).
    // The map is later rendered with `__sveltets_2_unionType(...)`.
    //
    // Direction matters:
    //   - component-then-DOM → DOM overwrites → DOM event only.
    //   - DOM-then-component → component appends → union(DOM, comp).
    //
    // Post-fix: merge DOM and component bubbles into ONE position-
    // ordered list per name. A DOM source REPLACES the entire list
    // (mirrors upstream's `set()`); a Component source APPENDS to
    // the list (mirrors upstream's `[].concat(exist, exp)`).
    //
    // This fixture exercises the component-then-DOM direction:
    // Inner (component) → button (DOM). The DOM bubble REPLACES the
    // prior component entry, so the wrapper's `$$Events.click` is
    // `MouseEvent` (the DOM event) only — not the intersection, and
    // not Inner's CustomEvent. (See fixture
    // 120-bubble-dom-then-component-union for the opposite direction.)
    import Inner from './Inner.svelte'
    let { label = '' }: { label?: string } = $props()
    void label
</script>

<Inner on:click />
<button on:click>{label}</button>
