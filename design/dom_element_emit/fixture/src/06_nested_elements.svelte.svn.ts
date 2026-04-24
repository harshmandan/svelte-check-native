/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 6 — nested elements across a non-trivial template. This
// mirrors the Slider.svelte shape (div > p + div > p + input) to prove
// that the scoped-block emit composes cleanly.
//
// Svelte source:
//   <div>
//       <p class="label">{field.label}</p>
//       <div class="container">
//           <p class="value">{value}</p>
//           <input oninput={(e) => ...} class="input" {value} type="range" />
//       </div>
//   </div>
//
// Emit shape (each element's `{ ... }` scope opens before children):
//   { svelteHTML.createElement("div", {});
//      { svelteHTML.createElement("p", { class: `label` }); field.label; }
//      { svelteHTML.createElement("div", { class: `container` });
//          { svelteHTML.createElement("p", { class: `value` }); value; }
//          { svelteHTML.createElement("input", { oninput: ..., class: `input`, value, type: `range` }); }
//      }
//   }

async function __svn_tpl_check() {
    const field: { label: string; key: string } = { label: "", key: "" };
    const value: string = "";
    const onchange = (_payload: { value: string }) => {};
    {
        svelteHTML.createElement("div", {});
        {
            svelteHTML.createElement("p", { class: `label` });
            field.label;
        }
        {
            svelteHTML.createElement("div", { class: `container` });
            {
                svelteHTML.createElement("p", { class: `value` });
                value;
            }
            {
                svelteHTML.createElement("input", {
                    oninput: (e) => onchange({ value: e.currentTarget.value }),
                    class: `input`,
                    value,
                    type: `range`,
                });
            }
        }
    }
    void field.key;
}

void __svn_tpl_check;

export {};
