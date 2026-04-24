// Minimal shim for __svn_ensure_type. Spec:
//   - Single-type form: accepts `T | null | undefined` as 3rd arg.
//   - Two-type form: accepts `T1 | T2 | null | undefined` as 4th arg.
// Returns an empty-object phantom so the expression-statement form
// `__svn_ensure_type(String, Number, expr);` is well-formed.
//
// Mirrors upstream svelte2tsx's `__sveltets_2_ensureType` at
// `language-tools/packages/svelte2tsx/svelte-shims-v4.d.ts:180-181`.
declare function __svn_ensure_type<T>(
    type: new (...args: any[]) => T,
    el: T | undefined | null,
): {};
declare function __svn_ensure_type<T1, T2>(
    type1: new (...args: any[]) => T1,
    type2: new (...args: any[]) => T2,
    el: T1 | T2 | undefined | null,
): {};
