// Consumer — proves contextual-type flow works with the class-wrapper.
//
// Component_default's props slot is now `ReturnType<Render<T>['props']>` =
// the body-scoped `$$Props` from $$render's scope. `typeof handler`
// resolves to `(item: T) => void` correctly, so when the consumer
// passes `handler={(val) => void val}` TS contextually types `val` as
// the generic `T` that `<T>` was bound to.
//
// Expected tsgo result: zero diagnostics.

import Component from './Component.js';
import type { ComponentProps } from 'svelte';

function $$render<MyItem>() {
    {
        // `T` on Component binds to `MyItem` here.
        const _: ComponentProps<typeof Component<MyItem>> = {
            // `val` is contextually typed as `MyItem` — not implicit any.
            handler: (val) => {
                const _check: MyItem = val;
                void _check;
            },
        };
        void _;
    }
}
$$render;
