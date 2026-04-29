<script lang="ts">
    // Round-9 follow-up #4: `{#each rows as { id, ...rest }}` —
    // the `rest` leaf should resolve to `Omit<element, 'id'>`.
    // Pre-fix native's pattern walker pushed nothing for the
    // ObjectPattern.rest branch, so the rest leaf inherited the
    // parent path (empty for top-level `{ id, ...rest }`) and
    // typed as the WHOLE element — a consumer using `rest.id`
    // would have wrongly succeeded (id was excluded but rest still
    // typed as the element).
    //
    // Upstream `slot.ts:111`'s `((${pattern}) => ${id})(unwrapArr
    // (items))` IIFE preserves the JS rest semantics — `rest` is
    // typed via destructure-projection. Native now mirrors with a
    // type-level `Omit<element, 'id'>` wrap via the new
    // `DestructureSeg::ObjectRest` segment.
    let { rows }: { rows: { id: number; label: string }[] } = $props()
</script>

{#each rows as { id, ...rest }}
    <slot {id} {rest} />
{/each}
