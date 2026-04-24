/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 5 — spread attribute `{...props}`.
//
// Svelte source:
//   <button {...spreadProps}>Click</button>
//
// Upstream emit: the spread expression goes directly into the attrs
// object as a spread element.
//
//   { svelteHTML.createElement("button", { ...spreadProps, }); }
//
// Spec: the spread object must satisfy the button's attrs slot via
// structural compatibility. Excess keys fire TS2353 as usual.

async function __svn_tpl_check() {
    const spreadProps: { type: "submit"; disabled: boolean } = {
        type: "submit",
        disabled: false,
    };
    {
        svelteHTML.createElement("button", {
            ...spreadProps,
        });
    }
}

void __svn_tpl_check;

export {};
