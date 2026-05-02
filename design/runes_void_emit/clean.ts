// Tsgo validation fixture for V5_PRIORITY Phase 4 (R-Conv #21).
//
// Validates that selectively emitting `;name;` (Ωignore-bracketed)
// only for $bindable() props — NOT for plain destructured ones —
// matches upstream svelte2tsx's `ExportedNames.ts:197-204` behaviour:
//
//   - bindable prop reassigned but never read → no TS6133 (saved by
//     the `;foo;` reference)
//   - non-bindable prop never read → TS6133 fires
//
// This file is the "all props correctly used" case: foo (bindable)
// and foo2 (non-bindable) are BOTH read in the body. No 6133s expected.

declare function $bindable<T>(fallback?: T): T;
declare function $props<P>(): P;

(async () => {
    type $$ComponentProps = { foo?: number; foo2: number };

    let {
        foo = $bindable(),
        foo2,
    }: $$ComponentProps = $props();
    /*Ωignore_startΩ*/foo;/*Ωignore_endΩ*/

    function onClick() {
        foo = 42;
    }

    // Read foo2 in body — keeps 6133 from firing.
    onClick;
    console.log(foo2);
})();
export {};
