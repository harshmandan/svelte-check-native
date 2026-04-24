// Child component WITH event bubbling — Props widened to
// Record<string, any>. Mirrors what we emit for components that
// have `on:X` bare-bubble directives on sub-components.
// Consumer can pass any props without TS2322 / TS2353.

declare const __svn_component_default: import('svelte').Component<Record<string, any>>;
declare type __svn_component_default = import('svelte').SvelteComponent<Record<string, any>>;
export default __svn_component_default;
