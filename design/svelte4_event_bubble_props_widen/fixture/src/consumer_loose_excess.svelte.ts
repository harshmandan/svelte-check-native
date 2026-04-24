// Consumer of child_loose passes an excess prop AND a type-mismatched
// required prop. Expect 0 diagnostics (Props is Record<string, any>).

import type ChildLoose from './child_loose.svelte.ts';
import { SvelteComponent } from 'svelte';

type Props = ChildLoose extends SvelteComponent<infer P> ? P : never;
declare const Child: new (o: { target?: any; props?: Props }) => SvelteComponent<Props>;

function $$render() {
    const inst = new Child({
        props: {
            anything: 42,
            nullable: null,
            mismatched_callback: (x: string, y: number) => {},
        },
    });
    void inst;
}
void $$render;
