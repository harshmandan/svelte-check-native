// Alternative shape using the `Awaited<ReturnType<$$render>>['props']`
// extraction pattern — same as our JS-overlay path uses today. Proves
// that this shape also restores contextual typing for the TS-overlay
// branch without having to duplicate the Props type expression.
//
// Why this matters for our port: our emit already synthesises a
// render function body. Returning a typed object literal (with
// `props`, `events`, `slots`, `exports`, `bindings` fields) lets the
// default-export declaration extract Props via
// `Awaited<ReturnType<typeof $$render>>['props']`, which is what
// upstream's `__sveltets_2_isomorphic_component($$render())` boils
// down to.

async function $$render_awaited() {
    let bar!: Object;
    let onclick!: ((e: MouseEvent) => void) | undefined;
    let onpointerenter!: ((e: PointerEvent) => void) | undefined;
    return {
        props: {
            bar: bar,
            onclick: onclick,
            onpointerenter: onpointerenter,
        },
        events: {},
        slots: {},
        exports: {},
    };
}

type __Bar_awaited_Props = Awaited<
    ReturnType<typeof $$render_awaited>
>['props'];

declare const Bar_awaited: import('svelte').Component<__Bar_awaited_Props, {}>;

async function consumer_awaited() {
    const __svn_C = __svn_ensure_component(Bar_awaited);
    new __svn_C({
        target: __svn_any(),
        props: {
            bar: {},
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
void consumer_awaited;
