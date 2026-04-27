// Validates the emit shape we WANT for `{#key EXPR}` blocks.
//
// `{#key EXPR}body{/key}` — Svelte re-creates the body when EXPR
// changes. From the type-checker's perspective there's no introduced
// binding, but EXPR itself should be type-checked just like any other
// interpolation expression — so a typo or out-of-scope reference
// surfaces as TS2304 / TS2552 rather than silently passing.
//
// Expected emit shape (matching how mustache_tag.rs handles `{expr}`):
//
//   ;(EXPR);
//   { /* body walk */ }
//
// Run: tsgo --pretty false -p tsconfig.json
//
// EXPECTED: 1 error — TS2304 on `undefinedFoo`. The `definedItem`
// reference is fine.

declare const definedItem: { id: string };

;(definedItem.id);
{
    // body walk happens here; bindings inside the body don't escape
}

// Misspelled / out-of-scope identifier — should fire.
;(undefinedFoo);
{
}
