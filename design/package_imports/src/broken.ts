import Comp, { type Scope } from '#lib/Comp.svelte';
import { type Missing } from '#lib/Comp.svelte';

const brand: 'Other' = Comp.brand;
const scope: Scope = 'nope';
export const all: [Missing, string, string] = [0 as never, brand, scope];
