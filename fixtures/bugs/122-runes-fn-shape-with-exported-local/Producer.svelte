<script lang="ts">
    // Round-6 follow-up #3: pre-fix native's runes fn_component gate
    // blocked when the instance script declared exported locals
    // (`export function foo()` here), pushing the component onto the
    // `$$IsomorphicComponent` shape. Upstream's gate
    // (`addComponentExport.ts:343`) is just `isRunesMode() &&
    // !usesSlots && !events.hasEvents()` — exported locals don't
    // matter, they show up on the `exports` projection of the
    // returned `Component<P, X, B>`.
    //
    // The iso shape's `new(...)` ctor is what makes `(typeof Comp)[]`
    // user patterns fail TS2322 — a callable-only function
    // expression can't satisfy a `new` ctor signature. The fn shape
    // (Component<>, callable-only) is the parity-correct choice.
    let { id }: { id: string } = $props()
    void id

    export function foo(): boolean {
        return true
    }
</script>

<p>id={id}</p>
