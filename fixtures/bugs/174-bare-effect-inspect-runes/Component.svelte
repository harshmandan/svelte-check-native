<script lang="ts">
    // Bare $effect / $inspect forms — companions to fixture 170 (which
    // covers the dotted .pre/.root/.tracking/.with/.trace surface).
    //
    // Shim shapes being locked here:
    //   $effect(fn: () => void | (() => void)): void
    //   $inspect<T extends any[]>(...values: T): { with: (fn: …) => void }
    //
    // The cleanup-returning form must also type-check (the union return
    // in the shim allows it).

    const count = $state(0)
    const name = $state('alice')

    // Bare effect with no cleanup.
    $effect(() => {
        void count
    })

    // Bare effect with cleanup — the function in the closure types as
    // (() => void) per the shim's union.
    $effect(() => {
        void count
        return () => {
            // cleanup
        }
    })

    // Bare $inspect — the return value's `.with` is exercised in
    // fixture 170; here we just call it for its side effect (logging
    // in dev). The return is intentionally discarded.
    $inspect(count, name)
</script>

<p>{count} {name}</p>
