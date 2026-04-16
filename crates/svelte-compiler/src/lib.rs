//! Bridge to the Svelte compiler for compiler-warning diagnostics.
//!
//! Spawns a persistent `bun` worker (or `node` as fallback) that imports the
//! Svelte compiler, takes source files over stdin, returns JSON diagnostics
//! over stdout. One worker per `svelte-check-native` run, not per file.
