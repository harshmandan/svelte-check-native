<script lang="ts">
    import Inner from './Inner.svelte'

    function handleString(e: CustomEvent<string>): void {
        void e.detail
    }
    function handleNumber(e: CustomEvent<number>): void {
        void e.detail
    }
</script>

<Inner on:from_callback={handleString} />
<Inner on:from_try={handleNumber} />
<!-- Wrong shape — must fire TS2345. -->
<Inner on:from_callback={handleNumber} />
