<script lang="ts">
    // Round-15 follow-up #3: three holes in the slot-attr rewriter's
    // shadow-stack discipline.
    //
    // (a) ForOf / ForIn left-binding never collected. The body
    //     references inside `for (const row of items) { row.length }`
    //     wrongly rewrote `row` as the template-local from
    //     `{#each rows as row}` instead of the loop binding.
    //
    // (b) FunctionDeclaration name not hoisted. A `function row() {}`
    //     declaration inside a callback didn't push `row` to the
    //     shadow stack — subsequent `row()` references rewrote to the
    //     template-local form.
    //
    // (c) `var row = …` inside an inner block was treated as block-
    //     scoped (R14 #5 added block-snapshot/truncate). `var` is
    //     FUNCTION-scoped — the binding should outlive its enclosing
    //     block. Pre-fix references to `row` AFTER the block wrongly
    //     rewrote to the template local because the block-truncate
    //     popped the var.
    //
    // The fix hoists `var` decls and `FunctionDeclaration` names at
    // function-body entry (mirroring JS hoisting); for-of/for-in
    // collect the left-binding into the shadow stack at loop entry.
    type Row = { id: number; vals: number[] }
    let { rows }: { rows: Row[] } = $props()
</script>

{#each rows as row}
    <slot
        a={(() => {
            // (a) for-of left binding `row` shadows template `row`
            // inside the loop body. `row.length` is `string.length`
            // = number. Pre-fix `row` rewrote to template local
            // `Row` (no `.length` field) → TS2339.
            for (const row of ['a', 'b', 'c']) {
                const _: number = row.length
                void _
            }
            return row.id
        })()}
        b={(() => {
            // (b) function decl `row` shadows template local for the
            // whole callback body (JS hoisting). Post-fix the fn
            // name is pushed at function-body entry; pre-fix `row()`
            // rewrote to `(undefined as any as Row)()` → TS2349.
            function row() {
                return 42
            }
            const _: number = row()
            void _
            return 0
        })()}
        c={(() => {
            // (c) `var row` is function-scoped. Post-fix the
            // function-body hoister pushes it before any block-
            // boundary truncate; pre-fix the inner-block truncate
            // popped it and the trailing `row` reference rewrote to
            // template local Row → TS2322 against `number`.
            if (true) {
                var row = 99
            }
            const _: number = row
            void _
            return 0
        })()}
    />
{/each}
