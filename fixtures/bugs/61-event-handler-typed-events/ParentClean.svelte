<script lang="ts">
    import Child from './Child.svelte'

    function handleMyEvent(e: CustomEvent<{ id: number }>): void {
        const n: number = e.detail.id
        void n
    }
</script>

<!-- Handler signature matches Child.$$Events.myevent — must be clean. -->
<Child on:myevent={handleMyEvent} />
