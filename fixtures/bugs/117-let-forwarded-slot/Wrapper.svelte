<script lang="ts">
    // SlotHandler PLAN Stage 4: this Wrapper's pattern is the
    // canonical layerchart shape — a context component (Inner)
    // exposes a let-binding that the Wrapper re-exposes via its
    // own default slot.
    //
    // Pre-Stage-4 the slot-def collector dropped `tooltip`
    // entirely because `<Inner let:tooltip>` shadowed the
    // identifier and the resolver had no upstream-equivalent
    // rewrite. Consumers' `<Wrapper let:tooltip>` then typed
    // `tooltip` as `any` and any field access passed silently.
    //
    // Post-Stage-4 the let-binding resolver builds
    // `__SvnComponentSlots<typeof Inner>['default']['tooltip']`
    // for the slot-def's `tooltip` entry; consumer-side
    // `let:tooltip` carries the proper { x: number; y: number }
    // shape.
    import Inner from './Inner.svelte'
    export let label: string = ''
    void label
</script>

<Inner let:tooltip>
    <slot {tooltip} />
</Inner>
