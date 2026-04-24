// Broken consumer: passes `head={…}` which is NOT in the synthesised
// $$ComponentProps (destructure doesn't include it; upstream ignores
// the JSDoc Props in TS-source mode).
//
// Expected: exactly 1 error at position of `head` — TS2353 "Object
// literal may only specify known properties, and 'head' does not
// exist in type 'Props'" (or equivalent excess-prop message).

import type ComponentPreviewType from './ComponentPreview.svelte.ts';
import { SvelteComponent } from 'svelte';

type Props = ComponentPreviewType extends SvelteComponent<infer P> ? P : never;

declare const ComponentPreview: new (o: { target?: any; props?: Props }) => SvelteComponent<Props>;

function $$render() {
    const p = new ComponentPreview({
        props: {
            id: 'pp',
            view: 'small',
            head: 'some-html',  // excess prop — must fire TS2353
        },
    });
    void p;
}
void $$render;
