<script lang="ts">
    // Round-12 follow-up #4: dispatcher declarations nested inside
    // for/while/switch/try statements AND callback arguments
    // (`setTimeout(() => { ... })`). Pre-fix native's dispatcher
    // walkers handled VarDecl/Function/Block/If but missed
    // For/ForOf/ForIn/While/DoWhile/Switch/Try/Labeled statements
    // AND function-expression arguments to call expressions —
    // upstream's TS walker visits them all via `ts.forEachChild`.
    //
    // Native now mirrors the recursion. Inner here declares typed
    // dispatchers in a `setTimeout` callback AND inside a
    // `try` block — both must contribute to the events surface.
    import { createEventDispatcher } from 'svelte'

    setTimeout(() => {
        const _t = createEventDispatcher<{ from_callback: string }>()
        void _t
    }, 0)

    try {
        const _u = createEventDispatcher<{ from_try: number }>()
        void _u
    } catch {}
</script>

<button>x</button>
