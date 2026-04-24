/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 5 — plain string attribute values. Upstream emits these as
// template-literal strings (`"class":`label`,`) rather than string
// literals, presumably so that attribute-value expressions inside
// `class="{cond ? 'a' : 'b'}"` interpolate without a span-remap. A
// bare template literal with no interpolation is type-equivalent to
// the string literal for attribute binding.
//
// Svelte source:
//   <p class="label">{field.label}</p>
//
// Emit shape:
//   { svelteHTML.createElement("p", { "class": `label`, }); field.label; }

async function __svn_tpl_check() {
    const field: { label: string } = { label: "" };
    {
        svelteHTML.createElement("p", {
            class: `label`,
        });
        field.label;
    }
}

void __svn_tpl_check;

export {};
