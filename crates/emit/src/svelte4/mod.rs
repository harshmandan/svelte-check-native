//! Svelte 4 syntax compat — emit-layer rewrites, droppable as a unit.
//!
//! Each submodule is a self-contained source-text rewrite triggered from
//! the main emit pipeline via explicit `// SVELTE-4-COMPAT` markers. No
//! shared state with the Svelte-5 passes. When Svelte 4 is officially
//! retired:
//!
//! 1. `rm -rf crates/emit/src/svelte4/`.
//! 2. `grep -rn '// SVELTE-4-COMPAT' crates/emit/src/` — delete each
//!    callsite and the surrounding dispatch.
//! 3. Remove the `mod svelte4;` declaration in `lib.rs`.
//!
//! See `design/phase_g/DESIGN.md` for the full plan.

pub mod compat;
pub mod reactive;
