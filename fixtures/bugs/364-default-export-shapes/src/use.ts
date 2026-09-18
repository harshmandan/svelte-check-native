import Comp from './Comp.svelte';
import NoProps from './NoProps.svelte';
export const p: Parameters<typeof Comp>[1] = { $$events: {} };
export const q: Parameters<typeof NoProps>[1] = { $$events: {}, $$slots: {} };
export const r: Parameters<typeof NoProps>[1] = { foo: 1 };
