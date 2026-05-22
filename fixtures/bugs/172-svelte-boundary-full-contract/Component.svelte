<script lang="ts">
    // `<svelte:boundary>` end-to-end contract. The shim declares the
    // element as:
    //   onerror?: (error: unknown, reset: () => void) => void
    //   failed?:  Snippet<[error: unknown, reset: () => void]>
    //   pending?: Snippet
    //
    // Fixture 103 already locks `failed`'s err: unknown narrowing.
    // This fixture covers the other two surfaces: `onerror`'s
    // signature and `pending`'s no-arg shape, plus a no-boundary-attrs
    // case that must type-check clean.
</script>

<!-- A: no attrs — both snippets are optional, must type-check clean. -->
<svelte:boundary>
    <p>plain</p>
</svelte:boundary>

<!-- B: onerror with correctly-typed handler. -->
<svelte:boundary
    onerror={(error, reset) => {
        const _e: unknown = error
        const _r: () => void = reset
        void _e
        void _r
    }}
>
    <p>typed handler</p>
</svelte:boundary>

<!-- C: pending snippet — no params; bare arrow must work. -->
<svelte:boundary>
    <p>with pending</p>
    {#snippet pending()}
        <p>loading…</p>
    {/snippet}
</svelte:boundary>
