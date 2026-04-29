<script lang="ts">
    // Round-11 follow-up #5: computed-key destructure
    // (`{ [k]: value }`) — `value` should resolve to
    // `parent[typeof k]`. Pre-fix native skipped the segment
    // entirely so `value` inherited the parent path (typed as the
    // whole element).
    //
    // Upstream's IIFE `(({ [k]: value }) => value)(source)` lets TS
    // infer `value: source[typeof k]` natively. Native now mirrors
    // with a new `KeyTypeof(SmolStr)` segment rendered as
    // `[typeof <ident>]`.
    type Row = { id: number; label: string }
    let { rows, key }: { rows: Row[]; key: 'id' | 'label' } = $props()
</script>

{#each rows as { [key]: value }}
    <slot {value} />
{/each}
