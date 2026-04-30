<script lang="ts">
    // Round-14 follow-up #1: an untyped dispatcher decl hidden inside
    // an IIFE used as an `if`-statement TEST condition needs the
    // typed-events rewrite. Pre-fix the rewrite walker's
    // `IfStatement` arm only descended into the consequent and the
    // alternate — never the test — so the dispatcher local declared
    // inside the test's IIFE never reached the
    // `createEventDispatcher<__SvnCustomEvents<$$Events>>()`
    // splice. The internal `dispatch('name', detail)` then ran
    // through the lax `<{}>` inference and accepted any second arg
    // silently.
    //
    // Native now mirrors the analyzer's coverage: every dispatcher
    // walker descends into `s.test` via `statements_inside_function_
    // expr`, and the rewrite walker does the same.
    import { createEventDispatcher } from 'svelte'
    interface $$Events {
        save: CustomEvent<{ id: number }>
    }
    let _e: $$Events | undefined
    void _e

    if (
        (() => {
            const d = createEventDispatcher()
            // After the rewrite, `d` is typed as
            // `EventDispatcher<__SvnCustomEvents<$$Events>>`. The
            // dispatched DETAIL must match `{id: number}`. Calling
            // with the wrong shape MUST fire TS2353.
            d('save', { wrong: 'shape' })
            return true
        })()
    ) {
        void 0
    }
</script>

<button>x</button>
