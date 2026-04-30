<script lang="ts">
    // Round-15 follow-up #1: every dispatcher walker matched only
    // bare `Statement::VariableDeclaration` and skipped the
    // `Statement::ExportNamedDeclaration(Declaration::
    // VariableDeclaration)` form. Upstream
    // (`processInstanceScriptContent.ts:271`) walks each statement
    // via `ts.forEachChild`, so the export wrapper is invisible —
    // an exported dispatcher is processed identically to a bare
    // one.
    //
    // For the dispatcher_typing_rewrite walker specifically: pre-
    // fix `export const d = createEventDispatcher()` stayed
    // un-rewritten (the `<__SvnCustomEvents<$$Events>>` splice was
    // skipped) and internal `d('name', detail)` calls passed the
    // lax `<{}>` inference silently. Post-fix the export form
    // reaches the rewrite path identically to the bare form.
    import { createEventDispatcher } from 'svelte'
    interface $$Events {
        save: CustomEvent<{ id: number }>
    }
    let _e: $$Events | undefined
    void _e

    export const d = createEventDispatcher()
    function fire() {
        // After the rewrite, `d`'s `save` detail is `{id: number}`.
        // Calling with a wrong shape MUST fire TS2353.
        d('save', { wrong: 'shape' })
    }
    void fire
</script>

<button>x</button>
