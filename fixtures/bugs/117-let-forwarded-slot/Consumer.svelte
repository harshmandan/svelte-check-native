<script lang="ts">
    import Wrapper from './Wrapper.svelte'
</script>

<Wrapper let:tooltip>
    <p>x={tooltip.x}, y={tooltip.y}</p>
    <!-- Wrong-field access: `tooltip.z` doesn't exist on the
         resolved shape `{ x: number; y: number }`. Pre-fix this
         passed silently because `tooltip` typed as `any`.
         Post-Stage-4 it fires TS2339. -->
    <p>z={tooltip.z}</p>
</Wrapper>
