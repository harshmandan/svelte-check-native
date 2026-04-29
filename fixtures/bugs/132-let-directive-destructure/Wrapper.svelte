<script lang="ts">
    // Round-8 follow-up #2: `<Inner let:tooltip={{ x, y, label }}>`
    // destructures Inner's `tooltip` slot prop into local vars
    // `x`, `y`, and `label`. Pre-fix native left `slot_key_path` as
    // None for destructure leaves, so each var dropped to the
    // unresolved-shadow path. Consumer-side checks here pin the
    // typed projection.
    //
    // Upstream `slot.ts:resolveDestructuringAssignmentForLet` wraps
    // the destructure pattern around `getSingleSlotDef(component,
    // slotName).${letNode.name}` — at type level this collapses to
    // `__SvnComponentSlots[Inner][default]['tooltip']['x']` for `x`,
    // `['y']` for y, etc. Native now combines `directive_path` with
    // each leaf's `destructure_path`.
    import Inner from './Inner.svelte'

    function takesNumber(n: number): void {
        void n
    }
    function takesString(s: string): void {
        void s
    }
</script>

<Inner let:tooltip={{ x, y, label }}>
    {takesNumber(x)}
    {takesNumber(y)}
    {takesString(label)}
    <!-- swap arg types — must fire TS2345 -->
    {takesString(x)}
</Inner>
