// Consumer — reproduces the TS7006 we observe on layerchart BarChart.
//
// With the current emit shape above, Component_default's props slot is
// `Partial<{ handler: any }>`. When the consumer passes an arrow
// function, TS can't contextually type the arrow's parameter — the
// context is `any`. So `val` is inferred implicit-any.

import Component from './Component.js';
import type { ComponentProps } from 'svelte';

async function $$render<MyItem>() {
    // Emulates the template body of a consumer component.
    // Equivalent of:  <Component handler={(val) => void val} />
    {
        const __svn_C = Component<MyItem>;
        // Arrow parameter position — this is where TS7006 fires.
        const _: ComponentProps<typeof __svn_C> = { handler: (val) => void val };
        void _;
    }
}
$$render;
