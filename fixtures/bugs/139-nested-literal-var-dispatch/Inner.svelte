<script lang="ts">
    // Round-10 follow-up #3: a `const NAME = 'literal'` declared
    // inside a function body should still resolve a sibling
    // `dispatch(NAME)` to its literal name. Pre-fix native's
    // literal-var collection was top-level only — the dispatched-
    // name scan recursed but couldn't see the nested literal
    // binding, so `dispatch(EV)` → undefined → no event surface
    // contribution.
    //
    // Upstream's TS walker visits all variable declarations
    // (`ComponentEvents.ts:210`) and the dispatched-name scan uses
    // the full set. Native now mirrors that recursion in the
    // literal-var collection pass.
    import { createEventDispatcher } from 'svelte'
    const dispatch = createEventDispatcher()

    function fire() {
        const EV = 'save'
        dispatch(EV)
    }
    void fire
</script>

<button>x</button>
