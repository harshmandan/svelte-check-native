<script lang="ts">
    import Producer from './Producer.svelte';

    // Threlte-style instancing pattern. With our pre-fix iso shape,
    // these `Parameters<typeof Producer>` / `(typeof Producer)[]`
    // assignments fired TS2322 because the iso interface had a
    // `new(...)` ctor signature that the inner-arrow callable type
    // could not satisfy. With the fn-component shape (Svelte's actual
    // `Component<P, X, B>`, callable-only), they pass cleanly.
    const make = (id: string) => {
        void id;
        return (...args: Parameters<typeof Producer>) => {
            return Producer(...args);
        };
    };

    function asArray(): (typeof Producer)[] {
        return [make('a'), make('b')];
    }

    void asArray;
</script>
