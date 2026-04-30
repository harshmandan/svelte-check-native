<script lang="ts">
    // Round-14 follow-up #6 wasn't sufficient — the analyzer's
    // source-order walker `scan_statement_in_source_order` registers
    // untyped dispatcher bindings into `dispatcher_locals` only in
    // the top-level `VariableDeclaration` arm. When the binding
    // sits in a `for`-init slot
    // (`for (let d = createEventDispatcher(); …) { d('save', …) }`),
    // the inline VarDecl handling registered `literal_vars` but
    // SKIPPED the dispatcher-local registration, so subsequent
    // `d('save', …)` calls in the for-body ran with `dispatcher_locals`
    // empty and the dispatched name 'save' never reached the
    // events surface synthesis.
    //
    // Round-12 #4 added control-flow recursion for for/while/switch
    // bodies; Round-13 #6 added loop headers; Round-14 #2 closes the
    // remaining hole: the untyped-dispatcher-binding registration
    // logic from the top-level VarDecl arm now also runs for the
    // for-init slot's declarators.
    import { createEventDispatcher } from 'svelte'

    // A typed dispatcher declares `save` with detail `number`. A
    // SECOND untyped dispatcher in for-init dispatches the same name
    // 'save'. Post-fix `find_dispatched_event_names` sees `save` as a
    // dispatched name; the events surface override layer then
    // collapses 'save' to `CustomEvent<any>`. Pre-fix the for-init
    // binding was missed, the dispatched-name walk ignored the
    // 'save' call, and the typed source's `CustomEvent<number>` won
    // — Consumer's `(e: CustomEvent<string>)` handler then fired
    // TS2345.
    const _typed = createEventDispatcher<{ save: number }>()
    void _typed

    function init() {
        for (let d = createEventDispatcher(), i = 0; i < 1; i++) {
            d('save', i)
        }
    }
    void init
</script>

<button>x</button>
