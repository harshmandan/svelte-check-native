<script lang="ts">
    // Round-7 follow-up #2: native pre-fix used the BoundIdent's
    // local name as the slot-key, so `<Inner let:tooltip={tip}>`
    // resolved `tip` to `__SvnComponentSlots<typeof Inner>['default']
    // ['tip']` — but the slot's actual prop is named `tooltip`, not
    // `tip`. The local var `tip` then typed as `any` and any field
    // access passed silently. Upstream's
    // `handleScopeAndResolveForSlot.ts:55` uses `letNode.name` (the
    // directive name) as the slot key, regardless of how the
    // expression aliases the binding locally.
    //
    // Post-fix: native's BoundIdent carries `slot_key_path` on
    // let-directive bindings — for the alias form `let:tooltip={tip}`
    // the path is `["tooltip"]` (directive name), so `tip` resolves
    // to the proper `{ x: number; y: number }` shape and field
    // accesses type-check correctly.
    import Inner from './Inner.svelte'

    function takesPoint(p: { x: number; y: number }): void {
        void p
    }
    function takesNumber(n: number): void {
        void n
    }
</script>

<Inner let:tooltip={tip}>
    {takesPoint(tip)}
    <!-- Wrong shape: tip.x is number, not the whole {x,y} object -->
    {takesNumber(tip)}
</Inner>
