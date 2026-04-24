/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 3 — event handler attribute: `<button onclick={(e) => …}>`.
//
// Svelte source:
//   <button onclick={(e) => console.log(e.clientX)}>click</button>
//
// Emit shape:
//   { svelteHTML.createElement("button", { onclick: (e) => console.log(e.clientX), }); }
//
// Spec: `onclick`'s signature is `(event: MouseEvent & { currentTarget }) => any`,
// so `e` contextually types as MouseEvent. `.clientX` is valid.

async function __svn_tpl_check() {
    {
        svelteHTML.createElement("button", {
            onclick: (e) => console.log(e.clientX, e.currentTarget.disabled),
        });
    }
}

void __svn_tpl_check;

export {};
