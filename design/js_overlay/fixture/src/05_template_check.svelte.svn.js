/// <reference types="./svn_shims.d.ts" />

// Pattern 5 — async template-check function with bind:this casts.
//
// TS form was:
//   async function __svn_tpl_check() {
//       element = null as any as HTMLElementTagNameMap['div'];
//   }
//   void __svn_tpl_check;
//
// `as any as T` is TS-only syntax. JS-overlay rewrite: a JSDoc
// double-cast — `/** @type {T} */ (/** @type {any} */ (null))`.

let element = $state();

async function __svn_tpl_check() {
    element =
        /** @type {HTMLElementTagNameMap['div']} */ (
            /** @type {any} */ (null)
        );
}

void __svn_tpl_check;
void element;
