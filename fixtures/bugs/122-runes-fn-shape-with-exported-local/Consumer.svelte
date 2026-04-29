<script lang="ts">
    import Producer from './Producer.svelte'

    // Threlte-style instancing pattern. With native's pre-round-6 gate,
    // a runes component that exported any instance-script local
    // (here, `export function foo()` on Producer) was pushed onto the
    // iso shape — `(typeof Producer)[]` then fired TS2322 because the
    // iso interface's `new(...)` ctor signature could not be satisfied
    // by a callable-only function expression. With the upstream-aligned
    // gate (drop the exported-locals exclusion), Producer keeps the
    // fn-component shape (Svelte's `Component<P, X, B>`, callable-only)
    // and the assignment passes.
    function asArray(): (typeof Producer)[] {
        return [Producer]
    }
    void asArray
</script>
