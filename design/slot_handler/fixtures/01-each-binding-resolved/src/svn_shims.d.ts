// Hand-written tsgo-validation fixture for slot-handler slice 1.
//
// Validates the shape of the resolved slot-def expression for slot
// attrs whose source identifier is shadowed by an `{#each items as
// item, i}` binding. Pre-implementation per Architecture rule #8:
// the emit shape and the consumer side must tsgo-clean here BEFORE
// any Rust port lands in the analyze + emit crates.
//
// Helper: `__svn_any<T>()` produces a value of type `T` for type-only
// channels. Used in the consumer fixture's slot-let destructure.

declare function __svn_any<T = any>(): T;
