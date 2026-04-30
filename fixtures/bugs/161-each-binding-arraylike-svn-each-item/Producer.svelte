<script lang="ts">
    // Round-14 follow-up #4: the each-block slot-attr resolver in
    // `template_walker.rs` projected the iterable element type via
    // an inline `T extends Iterable<infer __svn_T> ? __svn_T : never`.
    // For an ArrayLike-only items source (`{ length: N, [n]: U }`
    // — no `Symbol.iterator`), the inline projection collapses to
    // `never` and any slot binding derived from `row.<member>`
    // becomes `never.<member>` = `never`. Post-fix the resolver
    // routes through the `__SvnEachItem<typeof items>` shim
    // (`crates/typecheck/src/svelte_shims_core.d.ts:343`), which
    // distinguishes ArrayLike before falling back to Iterable —
    // so `row` resolves to the ArrayLike's `U` and the slot
    // binding's projected type is concrete.
    //
    // Producer-side iteration already routes through
    // `__svn_each_items(rows)` (which returns
    // `Iterable<__SvnEachItem<T>>`), so the `{#each}` block body
    // type-checks regardless. The resolver-path fix matters only
    // for slot-attr expressions whose VALUE is a reference into
    // the each scope.
    type Row = { id: number; vals: number[] }
    interface MyArrayLike {
        readonly length: number
        readonly [n: number]: Row
    }
    let { rows }: { rows: MyArrayLike } = $props()
</script>

{#each rows as row}
    <slot doubled={row.id} />
{/each}
