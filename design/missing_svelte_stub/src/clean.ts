import Gone, { Named, "a-b" as AB } from './Gone.svelte';
import * as Ns from './Gone.svelte';
import type TypeOnly from './Gone.svelte';
import { Thing } from '$lib/Gone.svelte';
import Bare from 'nopkg/Gone.svelte';

export type T1 = Gone;
export type T2 = Named;
export type T3 = TypeOnly;
export type T4 = Thing;
export type T5 = Bare<string>;
export type T6 = Ns.Member.Deep;
export type T7 = import('./Gone.svelte').Thing;
export const n: number = Gone;
export const c: string = Gone();
export const v = [Named, Ns.thing, Ns['x'], Thing, Bare, AB];
export { default as Again, Named as Renamed } from './Gone.svelte';
export * from './Gone.svelte';
export * as All from './Gone.svelte';
export async function lazy() {
    return (await import('./Gone.svelte')).default;
}
