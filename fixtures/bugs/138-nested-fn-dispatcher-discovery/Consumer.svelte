<script lang="ts">
    import Inner from './Inner.svelte'

    function takesString(e: CustomEvent<string>): void {
        void e.detail
    }

    // Wrong shape: Inner's `foo` event has `detail: string`. A
    // handler typed for number must fire TS2345.
    function takesNumber(e: CustomEvent<number>): void {
        void e.detail
    }
</script>

<Inner on:foo={takesString} />
<Inner on:foo={takesNumber} />
