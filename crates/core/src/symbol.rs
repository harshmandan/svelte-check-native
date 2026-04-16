//! String interning.
//!
//! Phase-1 choice: re-export `smol_str::SmolStr`. It inlines strings up to
//! 23 bytes without heap allocation, which covers nearly every identifier in
//! a Svelte component. Cheap `Clone`, `Eq` is a memcmp.
//!
//! If benchmarks later show identifier-equality is a hot path, we'll swap for
//! a `u32` symbol table keyed by a global interner. The re-export keeps
//! migration mechanical — users see `Symbol`, not `SmolStr`.

pub use smol_str::SmolStr as Symbol;
