/// <reference types="./svn_shims.d.ts" />

// Pattern 2 — JSDoc Props typedef + cast on $props destructure.
//
// TS form was:
//   interface $$Props { value?: string; count?: number }
//   let { value = '', count = 0 }: $$Props = $props();
//
// JS-overlay rewrite uses a JSDoc @typedef + a JSDoc @type cast on the
// destructured object literal. tsgo binds the prop types via JSDoc, so
// `count.toFixed(2)` is well-typed and `value.toFixed(2)` would fire.

/**
 * @typedef {Object} $$Props
 * @property {string} [value]
 * @property {number} [count]
 */

/** @type {$$Props} */
let { value = '', count = 0 } = $props();

void value.charAt(0); // OK — value is string
void count.toFixed(2); // OK — count is number
