<script lang="ts">
    import Producer from './Producer.svelte'

    // Threlte-style instancing — requires Producer's default export
    // to be the callable-only `Component<P, X, B>` shape (the iso
    // interface's `new(...)` ctor signature breaks the assignment
    // here with TS2322).
    function asArray(): (typeof Producer)[] {
        return [Producer]
    }
    void asArray
</script>
