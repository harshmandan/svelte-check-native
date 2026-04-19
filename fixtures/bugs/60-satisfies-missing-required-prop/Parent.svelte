<script lang="ts">
    import Child from './Child.svelte'
</script>

<!-- `label` is declared required in Child.svelte; omitting it must fire
     TS2741 at the `<Child />` tag position (column 1, the opening `<`).
     Before v0.3 Item 2 (phase 5 satisfies re-land + TokenMapEntry source
     map for satisfies trailers) this diagnostic fired in the
     synthesized emit region and got dropped by the source-map filter;
     this fixture locks the surfaced shape. -->
<Child />
