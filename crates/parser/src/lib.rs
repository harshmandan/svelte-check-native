//! Svelte 5 parser.
//!
//! Produces a `Document` AST where embedded JS/TS expressions are parsed into
//! real `oxc_ast` nodes — never stored as raw strings (that was the #1 source
//! of bugs in `upstream`).
//!
//! Handles Svelte 5 features from day one: runes, snippets, `{@attach}`,
//! `{@const}`, `{@render}`, all `svelte:*` special elements.
