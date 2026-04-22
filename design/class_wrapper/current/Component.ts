// Current emit shape (reduction).
//
// Reproduces the bare `$$render` + module-scope `declare const` pattern
// we emit today. The user's `interface $$Props { handler?: typeof handler }`
// resolves INSIDE `$$render_xyz<T>()` but can't be referenced at module
// scope without firing TS2304 — so the module-scope default-export
// declaration inlines the props type with `any` fallbacks for body-
// local refs.
//
// Expected tsgo result: TS7006 "Parameter 'val' implicitly has an 'any' type"
// on `<Component handler={(val) => void val} />`'s arrow parameter — because
// the prop context at the consumer site is `any`.

import type { Component as SvelteComponent } from 'svelte';

// Component.svelte — current emit shape (script_split + render body)
async function $$render_0000<T>() {
    // The user's Props interface lives INSIDE $$render so `typeof handler`
    // can resolve against the body-local `handler` below. Good for the
    // body's own type checks (nothing fires inside here).
    interface $$Props {
        handler?: typeof handler;
    }

    let handler: (item: T) => void = (_) => {};
    handler = undefined as any;

    // Pretend the template body checks happen here — not relevant to
    // this reduction.
    void (undefined as any as $$Props);
}
$$render_0000;

// Module scope — the default-export declaration.
//
// We can't mention `$$Props` here: it's scoped to $$render. If we say
// `handler?: typeof handler` at this level TS fires TS2304 because
// `handler` isn't in scope. So the fallback we currently use is to
// inline-expand the prop types with `any` where body-locals would
// otherwise be referenced.
declare const Component_default: <T>(
    __anchor: any,
    props: Partial<{ handler: any /* lost contextual flow */ }>,
) => any;
declare type Component_default<T> = SvelteComponent<
    Partial<{ handler: any }>
>;
export default Component_default;
