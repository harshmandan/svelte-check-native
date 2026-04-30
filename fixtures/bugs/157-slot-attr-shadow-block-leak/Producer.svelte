<script lang="ts">
    // Round-14 follow-up #5: a slot-attr expression whose callback
    // body contains a nested BLOCK with a `let` declaration that
    // shadows an outer template local. After the block, the body
    // references the OUTER template local — that reference should
    // still be rewritten as a template binding.
    //
    // Pre-fix native's `walk_statement_for_value_rewrite` pushed
    // bindings from `VariableDeclaration` onto the shadow stack but
    // never restored at scope boundaries. The inner `let row = 999`
    // in the block leaked, so the subsequent `row.id` reference was
    // treated as still-shadowed and the rewrite skipped. The bare
    // `row` then leaked to module scope (TS2304 if no module-level
    // `row` exists).
    //
    // Post-fix `BlockStatement` snapshots `shadowed.len()` on entry
    // and truncates on exit; the trailing `row.id` rewrites
    // correctly to `(undefined as any as (Row)).id`.
    type Row = { id: number; vals: number[] }
    let { rows }: { rows: Row[] } = $props()
</script>

{#each rows as row}
    <slot
        doubled={(() => {
            {
                let row = 999
                void row
            }
            return row.vals.map((x) => x + row.id)
        })()}
    />
{/each}
