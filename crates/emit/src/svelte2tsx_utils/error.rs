//! Error-formatting helpers used by the script-wrapping layer.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/utils/error.ts`.
//!
//! **Status: NA — we use Rust's `anyhow::Error` chains directly.**
//!
//! Upstream's `error.ts` provides a small abstraction over JS exception
//! shapes (collecting structured fields like file/line/column off
//! caught errors so they can be surfaced as diagnostics). The shape is
//! tied to JS's untyped error throwing.
//!
//! Our equivalent is Rust-idiomatic: each fallible function returns
//! `Result<T, anyhow::Error>` (in CLI/library entry points) or
//! `Result<T, Specific Error>` (in deeper layers). Errors propagate
//! via `?`, accumulate context via `.context(...)`, and surface to the
//! user via the CLI's top-level error printer (`crates/cli/src/main.rs`'s
//! `main()` boundary). There's no centralized error-formatting
//! module in our tree — the abstraction Rust gives via the
//! `std::error::Error` trait + `anyhow` covers the same ground.
//!
//! Cross-references:
//!
//! | Upstream concern | Our equivalent |
//! |---|---|
//! | Exception → diagnostic conversion | each layer that produces diagnostics constructs them directly via `svn_core::diagnostic::Diagnostic::error(...)` / `::warning(...)` |
//! | File/line/col attachment | structured into the `Diagnostic` shape itself; not derived from caught errors |
//! | Top-level error formatting | CLI's `main()` prints `anyhow::Error` chains via the `Display` impl |
//!
//! This file is a navigational stub only.
