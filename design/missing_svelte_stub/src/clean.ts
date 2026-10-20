import Gone, { Named } from './Gone.svelte';
import * as Ns from './Gone.svelte';
import type TypeOnly from './Gone.svelte';
import { Thing } from '$lib/Gone.svelte';
import Bare from 'nopkg/Gone.svelte';

export type T1 = Gone;
export type T2 = Named;
export type T3 = TypeOnly;
export type T4 = Thing;
export type T5 = Bare;
export const n: number = Gone;
export const c: string = Gone();
export const v = [Named, Ns.thing, Thing, Bare];
export { default as Again, Named as Renamed } from './Gone.svelte';
export * from './Gone.svelte';
export async function lazy() {
    return (await import('./Gone.svelte')).default;
}
