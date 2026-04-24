/// <reference path="./svelte_html_shim.d.ts" />

// Pattern 1 — class directive `class:foo={cond}` and shorthand `class:foo`.
//
// Svelte source:
//   <div class:active class:highlighted={isHighlighted}>
//
// Upstream emit shape: the createElement call does NOT include the
// class directive in the attrs object. Instead, the directive's
// value expression is emitted as a bare void statement inside the
// scoped block, after the createElement call. Type-checks the
// reference but doesn't constrain the attribute slot.
//
//   { svelteHTML.createElement("div", { }); active; isHighlighted; }

async function __svn_tpl_check() {
    const active: boolean = true;
    const isHighlighted: boolean = false;
    {
        svelteHTML.createElement("div", {});
        active;
        isHighlighted;
    }
}

void __svn_tpl_check;

export {};
