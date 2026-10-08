import Comp, { type Scope } from '#lib/Comp.svelte';
import { n } from '#lib/util.js';
import { fromIndex } from '#lib';

const brand: 'Comp' = Comp.brand;
const scope: Scope = 'org';
export const all = [brand, scope, n, fromIndex];
