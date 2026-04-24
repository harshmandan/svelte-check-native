/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 4 — a slider-component shape. A destructured event
// param that reaches for `target.value`.
//
// Svelte source:
//   <input oninput={({ target }) => onchange({ value: target.value })} type="range" />
//
// Emit shape:
//   { svelteHTML.createElement("input", {
//        oninput: ({ target }) => onchange({ value: target.value }),
//        type: "range",
//     });
//   }
//
// Spec: `oninput`'s signature is `(event: Event & { currentTarget: HTMLInputElement }) => any`.
// Destructuring `{ target }` pulls `target: EventTarget | null` from the base Event.
// EventTarget has NO `value` property → TS2339 fires on `target.value`.
//
// This file demonstrates the CORRECT pattern: reach for `currentTarget.value`
// (which resolves to HTMLInputElement.value, typed `string`).

async function __svn_tpl_check() {
    const onchange = (_payload: { value: string }) => {};
    {
        svelteHTML.createElement("input", {
            oninput: (e) => onchange({ value: e.currentTarget.value }),
            type: "range",
        });
    }
}

void __svn_tpl_check;

export {};
