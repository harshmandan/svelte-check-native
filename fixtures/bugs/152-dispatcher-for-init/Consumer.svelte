<script lang="ts">
    import Inner from './Inner.svelte'

    function takesNumber(e: CustomEvent<number>): void {
        void e.detail
    }
    function takesString(e: CustomEvent<string>): void {
        void e.detail
    }
</script>

<Inner on:tick={takesNumber} />
<!-- Wrong shape — must fire TS2345. -->
<Inner on:tick={takesString} />
