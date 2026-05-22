<script lang="ts">
    // Companion to 172 (clean) and 103 (failed-snippet narrowing):
    // proves the `<svelte:boundary onerror>` contract is enforced
    // by firing a deliberate wrong-arg-type mismatch. The shim
    // declares `onerror?: (error: unknown, reset: () => void) => void`;
    // a `(error: number, reset: number) => void` is not assignable.

    const wrong: (error: number, reset: number) => void = (_e, _r) => {}
</script>

<svelte:boundary onerror={wrong}>
    <p>boundary</p>
</svelte:boundary>
