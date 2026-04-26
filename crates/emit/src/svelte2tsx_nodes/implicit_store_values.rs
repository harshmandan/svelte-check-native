//! Implicit-store auto-subscribe: detect `$store` references in a
//! script body and mark `store` as a Writable/Readable for typing.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/nodes/ImplicitStoreValues.ts`.
//!
//! **Status: handled by `analyze::store`.**
//!
//! Upstream's `ImplicitStoreValues` walks the script AST collecting
//! `$NAME` references (Svelte's auto-subscribe sigil), then ensures
//! the underlying `NAME` local is typed as a store
//! (`Writable<T>` / `Readable<T>`) so `$NAME` reads compile against the
//! unwrapped value type. Output is consumed by the render-body emit
//! to add definite-assignment / type-narrow scaffolding.
//!
//! Our equivalent lives in [`svn_analyze::find_store_refs_with_bindings`]
//! (`crates/analyze/src/store.rs`). The walker collects `$NAME`
//! sigils with the underlying binding's source span, the emit
//! consumes the result via:
//!
//! - [`crate::svelte4::compat::rewrite_definite_assignment_in_place`] —
//!   adds `!` to the underlying `let NAME: Writable<T>;` declaration
//!   so TS flow doesn't fire TS2454 on subsequent `typeof NAME` reads.
//! - The default-export shape's `Awaited<ReturnType<typeof
//!   $$render>>['exports']` projection naturally preserves the
//!   `Writable<T>` shape via the body-local `typeof <name>` reference.
//!
//! No separate "ImplicitStoreValues" struct exists in our tree; the
//! analysis output is a `Vec<SmolStr>` of base names, threaded through
//! `apply_script_body_rewrites` in `lib.rs`.
//!
//! This file exists for parity navigation.
