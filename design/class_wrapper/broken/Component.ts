// Target emit shape (reduction) — upstream's class-wrapper pattern.
//
// Body-local `typeof handler` in `$$Props` resolves inside $$render's
// scope (same as current). The difference: at module scope we declare
// a `class __sveltets_Render<T>` whose methods *return* values from
// `$$render<T>()`. `ReturnType<Render<T>['props']>` extracts the body-
// scoped Props type THROUGH the render function — TS resolves the
// return type at the point $$render was declared, inside its own
// scope where `handler` is visible.
//
// Expected tsgo result: zero diagnostics.

import type {
    SvelteComponent,
    ComponentConstructorOptions,
} from 'svelte';

// --- Body of the component (same as before) ---
function $$render<T>() {
    interface $$Props {
        handler?: typeof handler;
    }

    let handler: (item: T) => void = (_) => {};

    return {
        props: null as unknown as $$Props,
        events: {} as {},
        slots: {} as {},
    };
}

// --- Class-wrapper: the bridge that lets module-scope code reach the
//     body-scoped Props type via ReturnType<…>. ---
class __sveltets_Render<T> {
    props()  { return $$render<T>().props; }
    events() { return $$render<T>().events; }
    slots()  { return $$render<T>().slots; }
    bindings() { return ''; }
    exports()  { return {}; }
}

// --- Isomorphic component interface: usable as both a constructor
//     (`new Component(...)`) and a call expression (`Component(...)`). ---
interface $$IsomorphicComponent {
    new <T>(
        options: ComponentConstructorOptions<
            ReturnType<__sveltets_Render<T>['props']> & { children?: any }
        >,
    ): SvelteComponent<
        ReturnType<__sveltets_Render<T>['props']>,
        ReturnType<__sveltets_Render<T>['events']>,
        ReturnType<__sveltets_Render<T>['slots']>
    >;
    <T>(
        internal: unknown,
        props: ReturnType<__sveltets_Render<T>['props']> & {
            children?: any;
        },
    ): ReturnType<__sveltets_Render<T>['exports']>;
}

const Component_default: $$IsomorphicComponent = null as any;
type Component_default<T> = InstanceType<typeof Component_default<T>>;
export default Component_default;
