// Consumer of child_strict passes an excess prop. Expect TS2353.

import type ChildStrict from './child_strict.svelte.ts';
import { SvelteComponent } from 'svelte';

type Props = ChildStrict extends SvelteComponent<infer P> ? P : never;
declare const Child: new (o: { target?: any; props?: Props }) => SvelteComponent<Props>;

function $$render() {
    const inst = new Child({
        props: {
            required: 'ok',
            extra: 'nope', // excess — must fire TS2353
        },
    });
    void inst;
}
void $$render;
