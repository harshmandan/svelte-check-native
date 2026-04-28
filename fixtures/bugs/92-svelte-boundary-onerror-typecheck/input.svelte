<svelte:options runes />
<script lang="ts">
    // Reviewer item #6: `<svelte:boundary onerror={…} />` flows
    // through `svelteHTML.createElement("svelte:boundary", { onerror,
    // failed })` so the callback signature `(error: unknown, reset:
    // () => void) => void` is checked against svelte/elements'
    // `'svelte:boundary'` shape. Pre-fix the boundary fell through
    // to a bare `{` scope, dropping the onerror check entirely.
    //
    // Wrong handler signature: `(error: number) => void` doesn't
    // accept `unknown`. Must fire TS2322 at the directive's value
    // position.
    function wrongOnError(error: number): void {
        void error
    }
</script>

<svelte:boundary onerror={wrongOnError}>
    <span>boundary content</span>
</svelte:boundary>
