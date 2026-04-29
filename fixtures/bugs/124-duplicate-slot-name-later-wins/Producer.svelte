<script lang="ts">
    // Round-7 follow-up #4: when a component has multiple `<slot
    // name="x">` sites for the same name (or multiple bare `<slot>`
    // sites both defaulting to `'default'`), upstream's `slot.ts:279`
    // stores them in a Map keyed by slot name — `set(slotName, attrs)`
    // makes the LAST site win. Native pre-fix pushed every SlotDef in
    // walk order; the slots literal then carried duplicate keys
    // (`{ 'header': {...}, 'header': {...} }`) which compile but
    // produce a noisy diagnostic AND make the slot-let inference at
    // the consumer side unpredictable.
    //
    // This fixture has TWO `<slot name="header">` sites — the first
    // exposes `value: number`, the second exposes `value: string`.
    // Upstream parity dictates the consumer's `let:value` reads the
    // second site's `string`.
    let { _items }: { _items: number[] } = $props()
    void _items
</script>

<slot name="header" value={1} />
<slot name="header" value="late" />
