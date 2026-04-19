// Mirror of upstream's `__sveltets_2_isomorphic_component` default-export
// shape for a Svelte-4-style component with a required prop.
//
// Goal: locate the TypeScript construct that makes `new ... ({ props: {} })`
// against a class-shape constructor fire TS2741 directly (without a
// satisfies trailer and without contamination from an index-signature
// intersection). Proved here so the minimal Rust change is clear:
// switch our Component<P> default-export emit to a class-shape default
// whose constructor takes `ComponentConstructorOptions<Props>`.
//
// Adapted from `language-tools/packages/svelte2tsx/svelte-shims-v4.d.ts:278-290`:
//
//     interface __sveltets_2_IsomorphicComponent<Props, Events, Slots, Exports, Bindings> {
//         new (options: ComponentConstructorOptions<Props>): SvelteComponent<...> & ...;
//         (internal: unknown, props: Props & {$$events?, $$slots?}): Exports;
//         z_$$bindings?: Bindings;
//     }
//     declare function __sveltets_2_isomorphic_component<...>(
//         klass: {props: Props, ...}
//     ): __sveltets_2_IsomorphicComponent<Props, Events, Slots, Exports, Bindings>;

interface ComponentConstructorOptions<Props> {
    target: any;
    props?: Props;
}

interface IsomorphicComponent<
    Props extends Record<string, any> = any,
    Events extends Record<string, any> = any,
    Slots extends Record<string, any> = any,
> {
    new (options: ComponentConstructorOptions<Props>): {
        $$prop_def: Props;
        $$events_def: Events;
        $$slot_def: Slots;
    };
    (internal: unknown, props: Props): any;
}

declare function isomorphic_component<
    Props extends Record<string, any>,
    Events extends Record<string, any>,
    Slots extends Record<string, any>,
>(klass: { props: Props; events: Events; slots: Slots }): IsomorphicComponent<
    Props,
    Events,
    Slots
>;

// Child: Svelte-4-style with required prop `b`.
function $$render_jsdoc() {
    let b: any;
    void b;
    return { props: { b }, events: {}, slots: {} };
}
const Jsdoc__SvelteComponent_ = isomorphic_component($$render_jsdoc());
export default Jsdoc__SvelteComponent_;

// Parent: consumer with `<Jsdoc />` omitting required prop.
// Passing a plain `Jsdoc` to the ensure_component wrapper then
// `new` on the result — mimics the overlay emit.
function $$render_parent() {
    async function __svn_tpl_check() {
        {
            const __svn_C_1 = Jsdoc__SvelteComponent_; // standin for ensure_component
            new __svn_C_1({ target: null, props: {} });
        }
    }
    void __svn_tpl_check;
}
void $$render_parent;
