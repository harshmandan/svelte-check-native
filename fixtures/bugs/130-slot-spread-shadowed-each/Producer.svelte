<script lang="ts">
    // Round-8 follow-up #1: `<slot {...item}>` inside `{#each items
    // as item}` rewrites the spread expression's shadowed root
    // `item` to a TYPE-form (`__SvnComponentSlots`-style cast). The
    // emit path for `SlotAttr::Spread { expr: Resolved(Type(T)) }`
    // wrote nothing pre-fix, producing the invalid TS literal
    // `slots: { 'default': { ...() } }` — a parse error that aborts
    // tsgo's whole-file check (every consumer-side type lookup on
    // the slot then degrades to `any`).
    //
    // Post-fix: emit `...(undefined as any as (T))` so the spread is
    // syntactically valid AND carries the right element type.
    let { rows }: { rows: { id: number; label: string }[] } = $props()
</script>

{#each rows as row}
    <slot {...row} />
{/each}
