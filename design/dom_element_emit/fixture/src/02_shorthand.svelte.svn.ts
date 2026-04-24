/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 2 — shorthand attribute: `<input {value}>`.
//
// Svelte source:
//   <script lang="ts">
//       let { value }: { value: string } = $props();
//   </script>
//   <input {value} type="range" />
//
// Emit shape (what this file models): the shorthand expands to the same
// `name,` shape in the attrs object — upstream Element.ts.addAttribute
// emits just `name,` when no value is present.
//
//   { svelteHTML.createElement("input", { value, type: "range", }); }
//
// Spec: shorthand's identifier must resolve; its type must conform to
// the attribute slot (here `HTMLInputAttributes.value`).

async function __svn_tpl_check() {
    const value: string = "50";
    {
        svelteHTML.createElement("input", {
            value,
            type: "range",
        });
    }
}

void __svn_tpl_check;

export {};
