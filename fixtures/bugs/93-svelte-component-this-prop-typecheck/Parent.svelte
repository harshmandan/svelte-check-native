<svelte:options runes />
<script lang="ts">
    import Child from './Child.svelte'

    // Reviewer item #1b: `<svelte:component this={X} ...>` routes
    // through `__svn_ensure_component(X); new $$_C({ props: {…} })`.
    // Pre-fix the whole node went un-checked through a bare scope.
    // The wrong-type prop here (`value="not-a-number"`) must fire
    // TS2322 at the directive position.
    const cmp = Child
</script>

<svelte:component this={cmp} value="not-a-number" />
