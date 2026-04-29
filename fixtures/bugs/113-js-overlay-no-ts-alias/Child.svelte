<script strictEvents>
    // Reviewer follow-up #1 (Fix A): pre-fix the synthesised event-
    // type alias and the synthesised props-type alias both emitted
    // unconditionally — including in JS overlays, where TS-only
    // declaration syntax is invalid.
    //
    // This Child has no `lang="ts"` attribute, so the emit lands as
    // a JS overlay (`.svelte.svn.js`). Pre-fix the emit included
    // body-local TS aliases that were dead code AND syntactically
    // invalid in the JS overlay. Post-fix both aliases are gated on
    // `is_ts`, so JS overlays emit cleanly without them.
    //
    // The strictEvents narrowing for JS overlays is a separate
    // (deferred) concern: JS overlays currently fall through to the
    // lax events surface because JSDoc can't express the
    // mapped/conditional shape the aliases carry. Real upstream
    // parity for JS overlay events needs JSDoc-friendly equivalents
    // — out of scope here; this fix only stops the TS-syntax-in-JS
    // bug.
    import { createEventDispatcher } from 'svelte'
    const dispatch = createEventDispatcher()
    dispatch('foo')
</script>

<button>js-strict</button>
