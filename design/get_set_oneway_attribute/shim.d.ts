/// <reference path="node_modules/svelte/svelte-html.d.ts" />
// The get/set helper as the overlay shim declares it, with svelte2tsx's
// name as an alias of the same type.
declare function __svn_get_set_binding<T>(
    get: (() => T) | null | undefined,
    set: (t: T) => void,
): T;
declare const __sveltets_2_get_set_binding: typeof __svn_get_set_binding;
