//! Error-formatting helpers used by the script-wrapping layer.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/svelte2tsx/utils/error.ts`.
//!
//! **Status: NA — we use Rust's `anyhow::Error` chains directly.**
//!
//! Upstream's `error.ts` provides `throwError`, a throw-with-position
//! helper: its call sites (Generics, `processModuleScriptTag`) raise a
//! JS exception carrying file/line/column so a downstream layer can turn
//! it into a diagnostic. The shape is tied to JS's untyped error
//! throwing. We don't throw — at each of those sites we produce a
//! `Diagnostic::error(...)` directly instead.
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
//! | `positionAt` / `getLineOffsets` / `clamp` | `crates/core/src/position.rs` — `PositionMap` / `line_col_utf8` / line-starts binary search / clamp-to-end |
//!
//! This file is a navigational stub only.
