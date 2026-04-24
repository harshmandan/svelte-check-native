/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 3 — svelte:body / svelte:head / svelte:window / svelte:document.
//
// Svelte source:
//   <svelte:body class="foo" />
//   <svelte:head><title>{label}</title></svelte:head>
//   <svelte:window onclick={() => …} />
//   <svelte:document oncopy={() => …} />
//
// Upstream emit: the colon STAYS in the tag name literal — the
// IntrinsicElements catalog has `"svelte:body"`, `"svelte:head"`,
// `"svelte:window"`, `"svelte:document"` as string keys.
//
//   { svelteHTML.createElement("svelte:body", { "class": `foo`, }); }
//   { svelteHTML.createElement("svelte:head", {});
//       { svelteHTML.createElement("title", {}); label; }
//   }
//   { svelteHTML.createElement("svelte:window", { "onclick": () => {}, }); }
//   { svelteHTML.createElement("svelte:document", { "oncopy": () => {}, }); }

async function __svn_tpl_check() {
    const label: string = "";
    {
        svelteHTML.createElement("svelte:body", {
            class: `foo`,
        });
    }
    {
        svelteHTML.createElement("svelte:head", {});
        {
            svelteHTML.createElement("title", {});
            label;
        }
    }
    {
        svelteHTML.createElement("svelte:window", {
            onclick: () => {},
        });
    }
    {
        svelteHTML.createElement("svelte:document", {
            oncopy: () => {},
        });
    }
}

void __svn_tpl_check;

export {};
