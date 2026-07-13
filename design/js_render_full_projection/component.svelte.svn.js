// Hand-written mimic of the NEW JS-overlay shape for a component like
// language-tools' `component-with-getters.svelte`:
//
//     <script>
//         export function test() { return 1; }
//         export class Foo {}
//         export const bar = true;
//     </script>
//
// The render function returns the FULL { props, events, slots,
// bindings, exports } projection (upstream createRenderFunction.ts
// returns all five fields for JS and TS alike), in JS-safe JSDoc
// syntax, and the default export projects the exports surface into
// `Component`'s second type parameter so instance members type
// precisely at consumers instead of widening to `any`.

async function $$render_deadbeef() {
    function test() {
        return 1;
    }
    class Foo {}
    const bar = true;
    void test;
    void Foo;
    void bar;
    return {
        props: /** @type {Record<string, never>} */ ({}),
        events: /** @type {{ [evt: string]: CustomEvent<any> }} */ ({}),
        slots: {},
        bindings: /** @type {string} */ (''),
        exports: /** @type {{ test: typeof test; Foo: typeof Foo; bar: typeof bar; }} */ ({})
    };
}
$$render_deadbeef;
/**
 * @typedef {Awaited<ReturnType<typeof $$render_deadbeef>>['props']} __SvnDefaultProps
 */
/**
 * @typedef {Awaited<ReturnType<typeof $$render_deadbeef>>['exports']} __SvnDefaultExports
 */
/** @type {import('svelte').Component<__SvnDefaultProps, __SvnDefaultExports>} */
export const __svn_component_default = /** @type {any} */ (null);
/** @typedef {ReturnType<typeof __svn_component_default>} __svn_component_default */
export default __svn_component_default;
