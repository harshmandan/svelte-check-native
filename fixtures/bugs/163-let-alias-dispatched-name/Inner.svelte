<script lang="ts">
    // Round-15 follow-up #1: the source-order walker
    // `scan_statement_in_source_order` recorded literal-var aliases
    // (`const EV = 'save'`) only when the binding was `const`.
    // Upstream's `getVariableAtTopLevel` walks every
    // VariableDeclaration regardless of kind — `let EV = 'save';
    // dispatch(EV)` should resolve EV to 'save' for the
    // dispatched-name layer the same way `const` does.
    //
    // To exercise the round-8 #5 duplicate-collapse path: pair an
    // inline-typed dispatcher (`<{ save: number }>`) with an
    // untyped one that dispatches a `let`-aliased 'save'. Pre-fix
    // the let-aliased dispatch didn't register 'save' as a
    // dispatched name, so the duplicate-collapse pass kept the
    // typed source's `save: CustomEvent<number>`. Post-fix the
    // alias resolves and 'save' is overridden via the untyped
    // layer to `CustomEvent<any>`.
    import { createEventDispatcher } from 'svelte'

    let EV = 'save'
    void EV

    const _typed = createEventDispatcher<{ save: number }>()
    void _typed

    const dispatch = createEventDispatcher()
    function fire() {
        dispatch(EV)
    }
    void fire
</script>

<button>x</button>
