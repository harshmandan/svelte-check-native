// Target shape for fix #2. Match upstream svelte2tsx by putting
// the precise prop shape in the PROPS slot, not in Exports.
//
// Two alternative declarations, both correct:
//
//   Option A (simple Component<>, swap slot contents):
//     Component<{onclick: (e: MouseEvent) => void}, {exported bind-this fields}>
//
//   Option B (Awaited<ReturnType<$$render>>['props']):
//     The render function's return carries `{props: {...}, exports: {...}}`.
//     A typedef extracts the Props shape and feeds it into Component<>.
//     This is what our JS-overlay path already does correctly.
//
// Either way: the PROPS slot now carries the ACTUAL prop shape,
// and consumer arrows get contextual typing.

declare const Bar_fixed: import('svelte').Component<
    {
        bar: Object;
        onclick?: ((e: MouseEvent) => void) | undefined;
        onpointerenter?: ((e: PointerEvent) => void) | undefined;
    },
    // Exports slot — instance-level surface for `bind:this={x}.prop`
    // access. Empty for components that don't expose instance exports.
    {}
>;

async function consumer_fixed() {
    const __svn_C = __svn_ensure_component(Bar_fixed);
    new __svn_C({
        target: __svn_any(),
        props: {
            bar: {},
            // Contextual typing flows from Props. `e: MouseEvent` here.
            onclick: (e) => {
                const x: number = e.clientX;
                void x;
            },
            onpointerenter: (e) => {
                const t: string = e.pointerType;
                void t;
            },
        },
    });
}
void consumer_fixed;
