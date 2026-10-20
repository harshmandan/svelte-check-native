// @ts-check
/** @import { Piece } from './AtImportGone.svelte' */
/** @type {import('./JsDocGone.svelte').default} */
export let x;
/**
 * @typedef {{
 *   a: import('./MultiLineGone.svelte').A
 * }} M
 */
/**
 * @param {M} m
 * @param {Piece} p
 * @returns {import('./ReturnsGone.svelte')}
 */
export function f(m, p) { return [m, p]; }
