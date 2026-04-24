/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 4 — svelte:element this={tag}.
//
// Svelte source:
//   <svelte:element this={tag} class="dyn">{label}</svelte:element>
//
// Upstream emit: first arg is the TAG EXPRESSION directly (not a
// string literal) so TS checks the tag against the valid IntrinsicElements
// keys union.
//
//   { svelteHTML.createElement(tag, { "class": `dyn`, }); label; }

async function __svn_tpl_check() {
    const tag: "button" | "div" = "button";
    const label: string = "";
    {
        svelteHTML.createElement(tag, {
            class: `dyn`,
        });
        label;
    }
}

void __svn_tpl_check;

export {};
