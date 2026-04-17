// Simulated overlay emit for Switch.svelte. Hand-written to model the
// shape we want the emitter to produce.
//
// Source Svelte (conceptual):
//   <script lang="ts">
//       let { checked, onchange }: {
//           checked: boolean;
//           onchange: (event: { checked: boolean }) => void;
//       } = $props();
//   </script>
//   <button onclick={() => onchange({ checked: !checked })}>
//       {checked ? 'on' : 'off'}
//   </button>

async function $$render_switch() {
    let {
        checked,
        onchange,
    }: {
        checked: boolean;
        onchange: (event: { checked: boolean }) => void;
    } = $props();

    async function __svn_tpl_check() {
        // Template-check body. Real emit would type-check element attrs
        // here; for Phase A just reference the onclick handler's arg
        // shape to prove the render function closure sees typed props.
        const handler: () => void = () => onchange({ checked: !checked });
        void handler;
        void checked;
    }
    void __svn_tpl_check;
    void checked;
    void onchange;
}
$$render_switch;

// Default export: the component as a callable. First arg is an opaque
// anchor (consumers pass `__svn_any()`); second arg is the Props object.
// Return type is `any` — we don't model Svelte's component-instance
// handle for now; the only thing consumers care about is the props
// contract carried by the second parameter.
import { SvelteComponent as $$_SC } from 'svelte';

declare class __svn_component_default extends $$_SC<{
    checked: boolean;
    onchange: (event: { checked: boolean }) => void;
}> {
    constructor(options: { target?: any; props?: Partial<{
        checked: boolean;
        onchange: (event: { checked: boolean }) => void;
    }> });
}

export default __svn_component_default;
