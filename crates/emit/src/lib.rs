//! TypeScript emission for the Svelte → `.svelte.ts` pipeline.
//!
//! Pure function: `SemanticModel` → `EmitOutput`. No back-references to parse
//! or analyze. The `VoidRefRegistry` populated by analyze is consumed here to
//! emit a single consolidated `void (...)` block — avoids `-rs`'s per-feature
//! reactive `void x;` sprinkling.
//!
//! Helper types (`__svn_*`) live in `helpers.d.ts` as a real file, loaded via
//! `include_str!` and emitted once to the cache dir. Never inlined per-file.
