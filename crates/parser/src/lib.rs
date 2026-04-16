//! Svelte 5 parser.
//!
//! ### Current scope
//!
//! Structural-only: identifies top-level `<script>`, `<script context="module">`,
//! and `<style>` sections, plus byte ranges of template content between them.
//! Embedded JS/TS is handed off verbatim to the `analyze` crate (which will
//! invoke `oxc_parser` on it).
//!
//! Template AST — elements, attributes, directives, control-flow blocks —
//! is not yet implemented. [`Template`] carries only byte ranges for now.
//!
//! ### Design mandate (from todo.md §1.2)
//!
//! Embedded JS/TS is NEVER stored as raw `String` for later character-level
//! scanning — that was the #1 source of bugs in `-rs`. Every expression goes
//! through `oxc_parser` exactly once, at the boundary where this crate hands
//! script contents to `analyze`.

// Tests are allowed to panic loudly on setup failures.
#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod ast;
mod attributes;
mod blocks;
pub mod document;
pub mod error;
mod mustache;
mod scanner;
mod script;
mod sections;
mod template;

pub use ast::{
    AttrValue, AttrValuePart, Attribute, AwaitBlock, CatchBranch, Comment, Component, Directive,
    DirectiveKind, DirectiveValue, EachAsClause, EachBlock, Element, ElseIfArm, ExpressionAttr,
    Fragment, IfBlock, Interpolation, KeyBlock, Node, PlainAttr, ShorthandAttr, SnippetBlock,
    SpreadAttr, SvelteElement, SvelteElementKind, Text as TemplateText, ThenBranch,
    is_component_tag, is_void_element,
};
pub use document::{
    Document, ScriptAttr, ScriptContext, ScriptLang, ScriptSection, StyleSection, Template,
};
pub use error::ParseError;
pub use script::{ParsedScript, parse_script_body};
pub use sections::parse_sections;
pub use template::{parse_all_template_runs, parse_template};

// Re-export oxc essentials so downstream crates can work with the AST
// without taking direct oxc dependencies in every workspace member that
// just *consumes* parsed output.
pub use oxc_allocator::Allocator;
pub use oxc_ast;
pub use oxc_diagnostics::OxcDiagnostic;
