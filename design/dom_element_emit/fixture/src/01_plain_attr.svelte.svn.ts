/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 1 — plain attribute: `<button type={t} disabled={d}>`.
//
// Svelte source:
//   <script lang="ts">
//       let { type, disabled }: { type: "button"|"submit"; disabled: boolean } = $props();
//   </script>
//   <button {type} {disabled}>click</button>
//
// Emit shape (what this file models):
//   async () => {
//       { svelteHTML.createElement("button", { type, disabled, }); }
//   }
//
// Spec: the attrs object is literal-typed against `HTMLButtonAttributes`,
// so a valid `type` and `disabled` must typecheck to zero errors.

async function __svn_tpl_check() {
    const type: "button" | "submit" = "button";
    const disabled: boolean = false;
    {
        svelteHTML.createElement("button", {
            type,
            disabled,
        });
    }
}

void __svn_tpl_check;

export {};
