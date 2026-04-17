// Simulates a third-party Svelte-4-style class component (lucide-svelte,
// phosphor-svelte, bits-ui, etc). These are SvelteComponent subclasses
// from Svelte's perspective — constructible, not callable.
//
// Our emit needs a shape that consumers can use uniformly for both our
// own callable overlays and these class exports.

import { SvelteComponent } from 'svelte';

type LucideIconProps = {
    size?: number;
    color?: string;
    class?: string;
};

export default class LucideIcon extends SvelteComponent<LucideIconProps> {}
