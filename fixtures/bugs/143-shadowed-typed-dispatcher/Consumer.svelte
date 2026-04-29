<script lang="ts">
    import Inner from './Inner.svelte'

    function handle(e: CustomEvent): void {
        void e.detail
    }
</script>

<!-- 'bar' is contributed by the top-level untyped dispatcher's
     `dispatch('bar')` call. Pre-fix `bar` was dropped because the
     nested same-name typed dispatcher cancelled it out. -->
<Inner on:bar={handle} />
