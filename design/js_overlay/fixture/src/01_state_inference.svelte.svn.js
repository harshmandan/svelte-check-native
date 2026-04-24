/// <reference types="./svn_shims.d.ts" />

// Pattern 1 — load-bearing JS-vs-TS difference.
//
// Under TS-strict, `let x = $state([])` infers `never[]` and
// `let x = $state(null)` infers `null` literal — so iteration over x
// or truthy-narrow + use fires TS2339. Under JS-loose + checkJs:true +
// noImplicitAny:false, both widen to `any`.
//
// This is the CodeMirror-wrapper case on a CMS-style bench
// (62 errors → 0 just from the extension change). MUST stay
// clean in the .js fixture.

let xs = $state([]);
let t = $state(null);

if (t) clearTimeout(t);
for (const ev of xs) {
    if (ev.view) ev.view.destroy();
}
