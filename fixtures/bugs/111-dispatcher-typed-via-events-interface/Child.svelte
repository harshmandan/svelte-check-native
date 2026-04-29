<script lang="ts">
    // Reviewer follow-up #3b: when `interface $$Events` is declared
    // and `createEventDispatcher()` is called UNTYPED, upstream
    // svelte2tsx rewrites the call to
    // `createEventDispatcher<__SvnCustomEvents<$$Events>>()`. That
    // gives `dispatch('name', detail)` calls inside the component
    // typed checking against $$Events.
    //
    // Pre-fix our pipeline left the dispatcher untyped — only
    // consumers' `<Child on:NAME>` got narrowed (via the
    // synth_events path), but bad `dispatch(...)` calls inside the
    // component passed silently. Post-fix the new
    // `dispatcher_typing_rewrite` pass splices the type-arg in.
    import { createEventDispatcher } from 'svelte'

    interface $$Events {
        click: CustomEvent<{ id: number }>
    }
    let _e: $$Events | undefined
    void _e

    const dispatch = createEventDispatcher()

    // Wrong-shape dispatch: declared `{ id: number }`, passing
    // `{ wrong: 'string' }`. Post-fix this fires TS2322 / TS2353
    // on the second arg via the rewritten call's type signature.
    function fire() {
        dispatch('click', { wrong: 'string' })
    }
    void fire
</script>

<button>strict-dispatch</button>
