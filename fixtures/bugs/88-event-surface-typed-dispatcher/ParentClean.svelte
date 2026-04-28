<script lang="ts">
    import Child from './Child.svelte'

    function handle(e: CustomEvent<{ id: number }>): void {
        const n: number = e.detail.id
        void n
    }
</script>

<!-- Handler matches the wrapped event surface — must be clean. -->
<Child on:click={handle} />
