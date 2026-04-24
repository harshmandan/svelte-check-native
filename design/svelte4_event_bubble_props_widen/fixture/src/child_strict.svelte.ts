// Child component WITHOUT event bubbling — strict Props preserved.
// Mirrors our fn_component-equivalent emit for components without
// createEventDispatcher / without on:X bubbles.

type $$ComponentProps = {
    required: string;
    optional?: number;
};

declare const __svn_component_default: import('svelte').Component<$$ComponentProps>;
declare type __svn_component_default = import('svelte').SvelteComponent<$$ComponentProps>;
export default __svn_component_default;
