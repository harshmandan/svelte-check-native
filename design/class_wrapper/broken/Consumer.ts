// Broken companion — proves the class-wrapper shape still catches real
// prop-type errors at the expected source position.
//
// Shape is identical to fixed/Component.ts — the Consumer deliberately
// passes a handler whose parameter type is wrong for the component's
// expected prop signature.
//
// Expected tsgo result: exactly one TS2322 diagnostic at the `handler`
// property value, complaining that `(val: number) => void` isn't
// assignable to `(item: MyItem) => void`. If this fires on any other
// site, the class-wrapper shape is narrowing incorrectly.

import Component from './Component.js';
import type { ComponentProps } from 'svelte';

function $$render<MyItem extends string>() {
    {
        const _: ComponentProps<typeof Component<MyItem>> = {
            // Deliberate mismatch: handler declared as number →, but
            // Component<MyItem> (with MyItem extends string) expects
            // `(item: MyItem) => void`.
            handler: (val: number) => void val,
        };
        void _;
    }
}
$$render;
