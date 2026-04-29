<script lang="ts">
    import Inner from './Inner.svelte'

    function handle(e: CustomEvent): void {
        void e.detail
    }
</script>

<!-- 'save' resolved from nested const EV = 'save'. -->
<Inner on:save={handle} />
