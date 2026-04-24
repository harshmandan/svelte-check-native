/// <reference path="./svn_shims.d.ts" />
/// <reference path="./svelte_html_shim.d.ts" />

// Broken-companion file. Locks the diagnostic count and codes that
// tsgo MUST fire against the DOM-element-emit shape. If any of these
// stops firing, our shape silently regressed.
//
// Each labelled block is a Svelte source + its emit shape + the TS
// code we expect. Validated 2026-04-24 with tsgo — exactly 4
// diagnostics at these positions:
//
//   src/Errors.svn.ts(23,13): error TS2322 — button type literal union
//   src/Errors.svn.ts(31,46): error TS2339 — EventTarget.value
//   src/Errors.svn.ts(40,13): error TS2322 — input value boolean
//   src/Errors.svn.ts(49,13): error TS2353 — unknown attr onclick_outside

async function __svn_tpl_check() {
    // E1: TS2322 — `type: string` doesn't satisfy `"submit"|"reset"|"button"|null|undefined`.
    // Svelte: <button {type}>  where `type: string`.
    {
        const type: string = "button";
        svelteHTML.createElement("button", {
            type,
        });
    }

    // E2: TS2339 — `target: EventTarget | null`, EventTarget has no `.value`.
    // Svelte: <input oninput={({ target }) => target.value} />
    {
        svelteHTML.createElement("input", {
            oninput: ({ target }) => target!.value,
        });
    }

    // E3: TS2322 — `value: boolean` doesn't satisfy `string | number | null | undefined`.
    // Svelte: <input {value}>  where `value: boolean`.
    {
        const value: boolean = true;
        svelteHTML.createElement("input", {
            value,
        });
    }

    // E4: TS2353 — unknown attr `onclick_outside` is not in HTMLButtonAttributes.
    // Svelte: <button onclick_outside={handler}>  where the attribute isn't in the button's slot.
    {
        const handler = () => {};
        svelteHTML.createElement("button", {
            onclick_outside: handler,
        });
    }
}

void __svn_tpl_check;

export {};
