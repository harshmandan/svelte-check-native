/// <reference types="./svn_shims.d.ts" />

// Pattern 4 — store auto-subscribe.
//
// TS form was:
//   let $counter!: __SvnStoreValue<typeof counter>;
//   void $counter;
//
// `!:` definite-assign is TS-only syntax. JS-overlay rewrite: assign a
// JSDoc-cast initial value (`null` cast through `any`). The cast
// preserves the unwrapped store-value type so flow analysis on
// `$counter.toFixed(2)` works. We narrow once with `if ($counter !=
// null)` — without it, the JSDoc-typed null shows through.

const counter = {
    /** @param {(value: number) => void} run */
    subscribe(run) {
        run(0);
        return () => {};
    },
};

/** @type {__SvnStoreValue<typeof counter>} */
let $counter = /** @type {any} */ (null);

// Reference matches what the TS overlay's `void $counter; void counter;`
// dead-store keep-alive does.
void $counter;
void counter;
