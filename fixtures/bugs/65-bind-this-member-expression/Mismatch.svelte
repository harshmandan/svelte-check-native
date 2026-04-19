<script lang="ts">
    // Non-element member-expression target: refs.count is typed
    // `number`, which can't accept an `HTMLInputElement`. Must fire
    // TS2322 at the bind:this expression position.
    //
    // Cross-HTMLElement type distinctions (e.g. declaring
    // `HTMLDivElement` for a `<input>` bind target) don't fire
    // errors because all HTML*Element types are structurally
    // compatible in TypeScript's DOM lib — HTMLDivElement's only
    // non-inherited property (`align`) is optional, so
    // HTMLInputElement satisfies it structurally. Matches upstream
    // `svelte2tsx`'s `EXPR = element.name` semantics.
    //
    // Simple-identifier targets also skip the check: the
    // definite-assign rewrite (`el = undefined as any;`) widens
    // the variable's flow type to `any` before the check fires.
    // Documented as a scope-limited case in NEXT.md.
    const refs: { count?: number } = {}
</script>

<input bind:this={refs.count} />
