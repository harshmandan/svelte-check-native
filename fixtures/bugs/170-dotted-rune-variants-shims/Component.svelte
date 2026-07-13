<script lang="ts">
    // Exercises every dotted rune variant declared in our shim's
    // `declare namespace $state | $derived | $effect | $inspect |
    // $props` blocks, plus `$inspect(...).with(...)` (method on
    // return value, not a namespace member). The point is to lock
    // in the shape so a future shim regression — a missing variant
    // or a wrong generic — fires a tsgo diagnostic on this fixture.
    //
    // All reads happen inside reactive closures so the
    // `state_referenced_locally` lint (which fires on direct reads
    // of `$state.raw`-tracked values from non-reactive scope) stays
    // silent — this fixture is testing shim shapes, not lint scope.

    const raw_num = $state.raw(0)
    const raw_obj = $state.raw<{ deep: { x: number } }>({ deep: { x: 1 } })

    // $state.snapshot — generic preserves the input type. Annotating
    // the LHS as { deep: { x: number } } proves the return type
    // flows through.
    const snap_thunk: () => { deep: { x: number } } = () => $state.snapshot(raw_obj)

    // $derived.by — thunk's return type flows out. The local is NOT
    // named `derived`: a binding whose name matches the rune base
    // turns `$derived.by(...)` into a store subscription of that
    // binding (the identifier sits in a member access, not the
    // rune-decl call position), which correctly fires TS7022 on the
    // self-referential declaration. That collision behaviour has its
    // own fixture; this one stays a pure shim-shape lock.
    const derived_val = $derived.by<number>(() => raw_num + 1)

    // $effect.pre / .root / .tracking / .pending.
    $effect.pre(() => {
        const tracked: boolean = $effect.tracking()
        const pending: number = $effect.pending()
        void tracked
        void pending
        return () => {}
    })
    const dispose: () => void = $effect.root(() => () => {})

    // $inspect(...).with(fn) — method on the return value, not a
    // namespace member. The fn parameter must accept ('init' |
    // 'update') as the first arg.
    $inspect(raw_num, derived_val).with((type, n, d) => {
        const _t: 'init' | 'update' = type
        const _n: number = n
        const _d: number = d
        void _t
        void _n
        void _d
    })

    // $inspect.trace — namespace member.
    $inspect.trace('Component')

    // $props.id — namespace member returning string.
    const id: string = $props.id()

    void snap_thunk
    void dispose
    void id
</script>

<p>component</p>
