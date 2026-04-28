<svelte:options runes />
<script lang="ts">
    import Child from './Child.svelte'

    function wrong(e: CustomEvent<{ name: string }>) {
        void e.detail.name
    }
</script>

<!-- Wrong handler shape — declared `{ id: number }`, parent
     passes `{ name: string }`. The aliased `ced<T>()` must still
     produce strict event narrowing so this fires TS2345. -->
<Child on:click={wrong} />
