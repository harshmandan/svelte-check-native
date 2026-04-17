// Consumer that uses a class component from a third-party package.
// Mimics real-world `<LucideSettings size={16} />` usage.
//
// Tests whether our emit shape can call both our own callable overlays
// AND class-shaped third-party components uniformly. Zero errors
// expected.

import LucideIcon from './Lucide.svelte.ts';
import Switch from './Switch.svelte.ts';

async function $$render_classes() {
    async function __svn_tpl_check() {
        // __svn_ensure_component dispatches via overloads. Class
        // components pass through; callable components are wrapped in
        // a constructor. After wrapping, `new $$_C({target, props})`
        // works uniformly.
        {
            const $$_C0 = __svn_ensure_component(Switch);
            new $$_C0({ target: __svn_any(), props: { checked: true } });
        }
        {
            const $$_C1 = __svn_ensure_component(LucideIcon);
            new $$_C1({ target: __svn_any(), props: { size: 16, color: 'red' } });
        }
    }
    void __svn_tpl_check;
}
$$render_classes;

declare const __svn_component_default: any;
export default __svn_component_default;
