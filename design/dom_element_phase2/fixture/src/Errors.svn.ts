/// <reference path="./svelte_html_shim.d.ts" />

// Broken-companion file locking Phase 2 diagnostic expectations:
//
//   src/Errors.svn.ts: TS2322 for class:foo={nonBoolean}? Actually
//   class directives accept any value (truthy test), so they don't
//   fire on non-boolean. The only useful diagnostic at the directive
//   itself is "Cannot find name" for shorthand with unbound ident.
//
// Instead we exercise:
//   E1: TS2304 — class:foo shorthand references an undeclared name.
//   E2: TS2345 — style: value is neither string nor number.
//   E3: TS2322 — svelte:element this={X} where X is not a valid tag
//       union member (our shim's IntrinsicElements keys).
//
// Validated 2026-04-24 with tsgo — exactly 2 diagnostics:
//   src/Errors.svn.ts(26,9): error TS2304 — Cannot find name 'missing'.
//   src/Errors.svn.ts(35,43): error TS2345 — boolean → style: value.
//
// Production shim's `__svn_ensure_type` uses `unknown` for the value
// param (not the fixture's strict `T1 | T2 | null | undefined`),
// so E2's TS2345 fires in this fixture but NOT in production emit.
// That tradeoff documented in svelte_shims_core.d.ts — preventing
// TS7041 false positives on Svelte-4 `export let x = undefined`
// props (seen on a charting-lib bench's Canvas/Html/Svg layout
// components) takes priority over
// catching style-directive type bugs, which are rare.

async function __svn_tpl_check() {
    // E1: shorthand class directive references an undeclared name.
    // Svelte: <div class:missing>
    // Emit: active reference emitted as bare statement after
    //       createElement; undeclared name → TS2304.
    {
        svelteHTML.createElement("div", {});
        missing;
    }

    // E2: style: value must be string | number. A boolean here fires
    // TS2345 "Argument of type 'boolean' is not assignable…".
    // Svelte: <div style:color={isActive}>  where isActive: boolean
    {
        const isActive: boolean = true;
        svelteHTML.createElement("div", {});
        __svn_ensure_type(String, Number, isActive);
    }

    // E3: svelte:element with an invalid tag expression. `tag` is
    // typed as a string that's NOT in the IntrinsicElements keys
    // union, but since IntrinsicElements has a `[name: string]` index
    // signature, any string is valid — no diagnostic. Preserved
    // here for documentation; not a locked error.
    // (Error count below does NOT include an E3.)
}

void __svn_tpl_check;

export {};
