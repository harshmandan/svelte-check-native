// Models the overlay shape we (and upstream) emit for Svelte-4
// reactive `$:` statements when they're NOT a binary-assignment.
//
// User source has, e.g., `$: sequence.config({...})`. Both ours and
// upstream wrap this in `;() => {$: <expr>}`. tsgo sees the inner
// `$:` as a labeled statement with an unused label, fires TS7028.
//
// The arrow IIFE gives the expression a type-check context (we WANT
// tsgo to surface real type errors inside it) without requiring it
// to actually run. The `$:` label is incidental — it's just there
// because we're preserving the user's original source structure.
//
// Run:
//   tsgo --pretty false -p tsconfig.json
//
// EXPECTED before our filter: 1 TS7028 error.
// EXPECTED after our filter:  0 errors (filter drops $: labels).

declare const sequence: { config(opts: { audio: boolean }): void };

;() => { $: sequence.config({ audio: true }) };

;() => { $: console.log('reactive side-effect') };

void sequence;
