// Clean consumer: passes only props that ARE in the synthesised
// $$ComponentProps. Must type-check with zero diagnostics.

import type ComponentPreviewType from './ComponentPreview.svelte.ts';
import { SvelteComponent } from 'svelte';

type Props = ComponentPreviewType extends SvelteComponent<infer P> ? P : never;

declare const ComponentPreview: new (o: { target?: any; props?: Props }) => SvelteComponent<Props>;

function $$render() {
    const p = new ComponentPreview({
        props: {
            id: 'pp',
            code: { html: '', css: '' },
            view: 'small',
            loading: false,
            append: '',
        },
    });
    void p;
}
void $$render;
