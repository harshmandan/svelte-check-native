/// <reference types="./svn_shims.d.ts" />

// Pattern 6 — async render wrapper, generics stripped.
//
// TS form was:
//   async function $$render_<hash>() { /* user instance script */ }
//
// JS-overlay rewrite: same shape, no generics (JS source can't carry
// `<script generics="...">` — that's Svelte-5 runes / TS-only). The
// async keyword is preserved so `await` in the user script body
// parses, matching the existing TS-overlay convention.

async function $$render_abc123() {
    let value = 0;
    await Promise.resolve();
    return value;
}

$$render_abc123;
