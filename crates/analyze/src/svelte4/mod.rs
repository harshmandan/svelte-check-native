//! Svelte 4 syntax compat — self-contained submodule, droppable as a unit.
//!
//! Every Svelte-4-specific analysis helper lives here, not scattered into
//! the main passes. Callers in `lib.rs` / `template_walker.rs` hit these
//! through explicit `// SVELTE-4-COMPAT` marker comments so a future
//! grep finds every site.
//!
//! Removal playbook (when Svelte 4 is officially retired):
//! 1. `rm -rf crates/analyze/src/svelte4/`.
//! 2. `grep -rn '// SVELTE-4-COMPAT' crates/analyze/src/` — delete each
//!    callsite and the surrounding dispatch.
//! 3. Remove the `pub mod svelte4;` declaration in `lib.rs`.
//!
//! See `design/phase_g/DESIGN.md` for the full plan.

pub mod on_directive;
