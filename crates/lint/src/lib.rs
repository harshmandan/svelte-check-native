//! A11y and structural lint rules that don't require a type checker.
//!
//! Rules are keyed by a `RuleId` enum (not `&str`) for O(1) severity lookups.
//! Each rule implements `check(ctx, node)`. The coverage table is generated at
//! test time to prevent drift.
