/// <reference types="./svn_shims.d.ts" />

// Companion broken file — confirms tsgo still catches genuine errors
// under JS-loose + checkJs:true. If THIS file ever stops firing, the
// .svn.js overlay is silently swallowing diagnostics that should be
// reaching users.

/** @type {string} */
let label;
label = 42; // E1: TS2322 — Type 'number' is not assignable to type 'string'.
void label;

/** @param {string} s */
function takesString(s) {
    return s.length;
}
takesString(123); // E2: TS2345 — Argument of type 'number' …
