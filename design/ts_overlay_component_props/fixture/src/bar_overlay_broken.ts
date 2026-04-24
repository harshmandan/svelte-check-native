// Current shape of our TS-overlay default export. Mirrors the tail
// of a `Bar.svelte.svn.ts` overlay as emitted on a charting-lib
// bench:
//
//   declare const __svn_component_default: import('svelte').Component<
//       Record<string, any> & __SvnAllProps,  // PROPS slot — TOO LOOSE
//       { bar: Object; onclick: ((e: MouseEvent) => void) | undefined; ... }  // EXPORTS slot
//   >;
//
// With this shape, a consumer writing
//   new Bar({ props: { onclick: (e) => {} } })
// checks the `props` object against `Record<string, any>`. The
// `onclick` arrow gets NO contextual type, so `e` falls to `any` —
// fires TS7006 "Parameter 'e' implicitly has an 'any' type" in
// strict mode.

type __SvnAllProps = { [index: string]: any };

declare const Bar_current: import('svelte').Component<
    Record<string, any> & __SvnAllProps,
    {
        bar: Object;
        onclick: ((e: MouseEvent) => void) | undefined;
        onpointerenter: ((e: PointerEvent) => void) | undefined;
    }
>;

// Consumer site — mirrors a chart-bench route that instantiates
// <Bar onclick={(e) => ...}> inside an {#each} loop.
async function consumer_current() {
    const __svn_C = __svn_ensure_component(Bar_current);
    new __svn_C({
        target: __svn_any(),
        props: {
            bar: {},
            // Expected behaviour FROM OUR CURRENT EMIT: `e` falls to `any`
            // (because the Props slot is `Record<string, any>`). In strict
            // mode this fires TS7006.
            onclick: (e) => {
                e.clientX;
            },
            onpointerenter: (e) => {
                e.pointerType;
            },
        },
    });
}
void consumer_current;
