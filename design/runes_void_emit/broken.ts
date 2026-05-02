// Tsgo validation fixture for V5_PRIORITY Phase 4 (R-Conv #21).
//
// The broken case mirrors language-server's $bindable-reassign.v5
// fixture: foo is bindable + reassigned but never READ; foo2 is
// non-bindable and entirely unreferenced.
//
// Expected diagnostics:
//   - line 30, col 8: TS6133 'foo2' is declared but its value is
//     never read.
//   - NO TS6133 on `foo` — the `;foo;` reference at line 28 keeps
//     it alive.

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

    onClick;
})();
export {};
