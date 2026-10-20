import type { Component } from 'svelte';
import App from './App.svelte';
import Gone from './Gone.svelte';
export { default as Also } from './AlsoGone.svelte';
export * from './StarGone.svelte';
import type Typed from './TypedGone.svelte';
export async function load() {
    return import('./LazyGone.svelte');
}
export const all: Component[] = [App, Gone];
export type T = typeof Typed;
