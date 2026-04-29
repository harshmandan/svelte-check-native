<script lang="ts">
    // Round-7 follow-up #1: pre-fix native's slot-attr rewriter only
    // fired when the LEADING identifier of the expression was
    // shadowed. So `<slot value={item}>` was rewritten correctly,
    // but `<slot value={fmt(item)}>` (call with shadowed arg),
    // `<slot {item}>` (object shorthand if it had been wrapped, or
    // nested `{ wrap: item }`), and `<slot value={fallback ?? item}>`
    // (logical OR) all leaked the shadowed `item` to module scope —
    // resolving against whatever module-scope `item` was in scope, or
    // failing TS2304 if none was.
    //
    // Upstream's `slot.ts:resolveExpression` walks the WHOLE
    // expression AST and rewrites every relevant identifier in place.
    // Native now mirrors that with `rewrite_slot_attr_expr_value`:
    // each shadowed identifier (in non-member-property, non-object-
    // key, non-arrow-body position) gets replaced with
    // `(undefined as any as (TYPE))`; the surrounding expression
    // (call, ternary, logical, object literal) splices verbatim.
    let { rows }: { rows: { id: number; label: string }[] } = $props()
    function double(n: number): number {
        return n * 2
    }
</script>

{#each rows as item}
    <slot doubled={double(item.id)} wrapped={{ inner: item.label }} />
{/each}
