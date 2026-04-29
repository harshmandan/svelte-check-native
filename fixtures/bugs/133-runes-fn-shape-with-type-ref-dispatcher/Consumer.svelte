<script lang="ts">
    import Producer from './Producer.svelte'

    // Threlte-style instancing — requires the callable-only
    // `Component<P, X, B>` shape. Pre-fix native pushed Producer
    // onto the iso shape because of the typed dispatcher's TYPE-
    // REFERENCE arg (`<MyEvents>`); the iso `new(...)` ctor
    // signature can't be satisfied by a callable-only function
    // expression and this fired TS2322.
    function asArray(): (typeof Producer)[] {
        return [Producer]
    }
    void asArray
</script>
