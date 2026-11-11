//! Svelte 4 syntax compat — analyze-layer helpers, droppable as a unit.
//!
//! Each submodule models a Svelte-4-only template concept and is called
//! from the main analyze pipeline at explicit `// SVELTE-4-COMPAT`
//! markers. When Svelte 4 is officially retired:
//!
//! 1. `rm -rf crates/analyze/src/svelte4/`.
//! 2. `grep -rn '// SVELTE-4-COMPAT' crates/` — delete each callsite and
//!    the surrounding dispatch.
//! 3. Remove the `mod svelte4;` declaration in `lib.rs`.

pub(crate) mod slot_position;
