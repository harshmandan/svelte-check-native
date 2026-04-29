<script lang="ts">
    // Round-13 follow-up #2-rewrite: when `interface $$Events` is
    // declared, untyped `createEventDispatcher()` calls should be
    // rewritten to `createEventDispatcher<__SvnCustomEvents
    // <$$Events>>()` so internal `dispatch('name', detail)` calls
    // type-check against the declared event surface. Pre-fix the
    // rewrite was top-level only — a nested untyped dispatcher
    // inside `function init() { const d = createEventDispatcher() }`
    // stayed un-rewritten and `d('name', wrongShape)` calls passed
    // the lax `<{}>` inference silently.
    //
    // Native now mirrors upstream's `ComponentEvents.ts:130` walk
    // by recursing through Function/Block/If/For/While/Switch/Try
    // bodies + arrow/function args.
    import { createEventDispatcher } from 'svelte'
    interface $$Events {
        save: CustomEvent<{ id: number }>
    }
    let _e: $$Events | undefined
    void _e

    function init() {
        const d = createEventDispatcher()
        // After the rewrite, `d` is typed as
        // `EventDispatcher<__SvnCustomEvents<$$Events>>`. The
        // dispatched DETAIL must match `{id: number}`. Calling
        // with the wrong shape MUST fire TS2345.
        d('save', { wrong: 'shape' })
    }
    void init
</script>

<button>x</button>
