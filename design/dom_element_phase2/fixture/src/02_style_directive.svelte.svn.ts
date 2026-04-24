/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 2 — style directive `style:prop={value}` and shorthand `style:color`.
//
// Svelte source:
//   <div style:color style:padding={"8px"}>
//
// Upstream emit shape: the createElement call does NOT include the
// style directive in the attrs object. Each directive's value is
// passed through `__sveltets_2_ensureType(String, Number, value)` so
// the value is validated against `string | number | null | undefined`.
//
//   { svelteHTML.createElement("div", { });
//     __svn_ensure_type(String, Number, color);
//     __svn_ensure_type(String, Number, "8px");
//   }

async function __svn_tpl_check() {
    const color: string = "red";
    {
        svelteHTML.createElement("div", {});
        __svn_ensure_type(String, Number, color);
        __svn_ensure_type(String, Number, "8px");
    }
}

void __svn_tpl_check;

export {};
