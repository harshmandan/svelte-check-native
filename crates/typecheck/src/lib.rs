//! tsgo integration + tsconfig overlay.
//!
//! Generates the cache-dir overlay tsconfig (extends user tsconfig, adds
//! virtual `.svelte.ts` paths, pins build-info location), spawns `tsgo
//! --project <overlay>.json --pretty true --noErrorTruncation`, parses the
//! pretty output with:
//!
//! ```text
//! ^(.+):(\d+):(\d+) - (error|warning) TS(\d+): (.*)$
//! ```
//!
//! plus up-to-4-lines lookahead for the `~~~~` underline to recover span
//! length. Diagnostics are mapped from the generated `.svelte.ts` back to the
//! original `.svelte` via source maps.
//!
//! tsgo is the only backend. No tsc fallback.
