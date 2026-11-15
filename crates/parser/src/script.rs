//! oxc-backed script parsing.
//!
//! A thin wrapper around `oxc_parser` that gives downstream crates a
//! plain-shaped API: hand us the script body + language flag, get back a
//! `Program` plus any parse diagnostics. We expose the oxc types directly;
//! hiding them would be a lossy abstraction since `analyze` walks them
//! natively.
//!
//! **Architecture rule:** embedded JS/TS is parsed exactly once here,
//! into a real AST. Never scanned character-by-character downstream —
//! hand-rolled scanners are fragile by construction; an AST-level
//! pattern match makes whole classes of bug categorically impossible.

use oxc_allocator::Allocator;
use oxc_ast::ast::Program;
use oxc_diagnostics::OxcDiagnostic;
use oxc_parser::{Parser, ParserReturn};
use oxc_span::SourceType;

use crate::document::ScriptLang;

/// Outcome of parsing a single script body.
pub struct ParsedScript<'alloc> {
    pub program: Program<'alloc>,
    /// Any oxc-level syntax errors. Recoverable: `program` is still a valid
    /// AST (possibly with error nodes) even when `errors` is non-empty.
    pub errors: Vec<OxcDiagnostic>,
    /// Whether the parser recognized this as a TypeScript file.
    pub is_typescript: bool,
    /// Whether oxc flagged unrecoverable errors (panic'd).
    pub panicked: bool,
}

/// Parse a script body with oxc.
///
/// `content` must live at least as long as `allocator` — the returned
/// `Program` borrows from both.
pub fn parse_script_body<'alloc>(
    allocator: &'alloc Allocator,
    content: &'alloc str,
    lang: ScriptLang,
) -> ParsedScript<'alloc> {
    let mut source_type = SourceType::default().with_module(true);
    if lang == ScriptLang::Ts {
        source_type = source_type.with_typescript(true);
    }

    let ParserReturn {
        program,
        diagnostics,
        fatal_error,
        ..
    } = Parser::new(allocator, content, source_type).parse();

    ParsedScript {
        program,
        // oxc 0.137 renamed `ParserReturn.errors` to `diagnostics`
        // (a `Vec<OxcDiagnostic>` newtype); flatten back to the plain
        // vec our `ParsedScript` exposes.
        errors: diagnostics.into_vec(),
        is_typescript: lang == ScriptLang::Ts,
        panicked: fatal_error,
    }
}

/// Which of a component's two scripts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
    Instance,
    Module,
}

/// The parses of one component's scripts, shared by every pass that
/// reads them (lint, emit, the import checks): each script is parsed at
/// most once per language, when a pass first asks for it.
pub struct ScriptParses<'a> {
    allocator: &'a Allocator,
    instance: Option<&'a str>,
    module: Option<&'a str>,
    /// `[instance JS, instance TS, module JS, module TS]`.
    parsed: [std::cell::OnceCell<ParsedScript<'a>>; 4],
}

impl<'a> ScriptParses<'a> {
    pub fn new(allocator: &'a Allocator, doc: &crate::document::Document<'a>) -> Self {
        Self {
            allocator,
            instance: doc.instance_script.as_ref().map(|s| s.content),
            module: doc.module_script.as_ref().map(|s| s.content),
            parsed: Default::default(),
        }
    }

    /// The `kind` script's body parsed as `lang`; `None` when the
    /// component has no such script.
    pub fn get(&self, kind: ScriptKind, lang: ScriptLang) -> Option<&ParsedScript<'a>> {
        let (content, slot) = match kind {
            ScriptKind::Instance => (self.instance?, 0),
            ScriptKind::Module => (self.module?, 2),
        };
        let slot = slot + usize::from(lang == ScriptLang::Ts);
        Some(self.parsed[slot].get_or_init(|| parse_script_body(self.allocator, content, lang)))
    }

    /// Whether these are the parses of `doc`'s scripts: the same text,
    /// not just equal text.
    pub fn covers(&self, doc: &crate::document::Document<'_>) -> bool {
        let same = |held: Option<&str>, script: Option<&crate::document::ScriptSection<'_>>| match (
            held, script,
        ) {
            (None, None) => true,
            (Some(a), Some(b)) => std::ptr::eq(a, b.content),
            _ => false,
        };
        same(self.instance, doc.instance_script.as_ref())
            && same(self.module, doc.module_script.as_ref())
    }

    pub fn instance(&self, lang: ScriptLang) -> Option<&ParsedScript<'a>> {
        self.get(ScriptKind::Instance, lang)
    }

    pub fn module(&self, lang: ScriptLang) -> Option<&ParsedScript<'a>> {
        self.get(ScriptKind::Module, lang)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oxc_ast::ast::Statement;

    #[test]
    fn parses_empty_script_cleanly() {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, "", ScriptLang::Js);
        assert!(parsed.errors.is_empty());
        assert!(!parsed.panicked);
        assert!(parsed.program.body.is_empty());
    }

    #[test]
    fn parses_javascript_top_level_let() {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, "let x = 1;", ScriptLang::Js);
        assert!(parsed.errors.is_empty());
        assert_eq!(parsed.program.body.len(), 1);
    }

    #[test]
    fn parses_typescript_type_annotation() {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, "let x: number = 1;", ScriptLang::Ts);
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert!(parsed.is_typescript);
        assert_eq!(parsed.program.body.len(), 1);
    }

    #[test]
    fn js_mode_rejects_ts_type_annotation() {
        // `let x: number = 1;` is invalid JS. oxc reports a syntax error.
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, "let x: number = 1;", ScriptLang::Js);
        assert!(!parsed.errors.is_empty());
    }

    #[test]
    fn parses_import_statements() {
        let alloc = Allocator::default();
        let src = "import { writable } from 'svelte/store';";
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert_eq!(parsed.program.body.len(), 1);
        assert!(matches!(
            parsed.program.body[0],
            Statement::ImportDeclaration(_)
        ));
    }

    #[test]
    fn parses_rune_call_site() {
        // `$state(0)` must be parseable as a normal function call — runes
        // are lexically indistinguishable from `$`-prefixed function calls
        // at this layer.
        let alloc = Allocator::default();
        let src = "let count = $state(0);";
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
        assert_eq!(parsed.program.body.len(), 1);
    }

    #[test]
    fn parses_destructuring_props() {
        let alloc = Allocator::default();
        let src = "let { items = $bindable<Map<string, number>>(new Map()), onsubmit } = $props();";
        let parsed = parse_script_body(&alloc, src, ScriptLang::Ts);
        assert!(parsed.errors.is_empty(), "errors: {:?}", parsed.errors);
    }

    #[test]
    fn malformed_script_has_errors_but_returns_program() {
        let alloc = Allocator::default();
        let parsed = parse_script_body(&alloc, "let x = ;", ScriptLang::Js);
        assert!(!parsed.errors.is_empty());
        // Program is still present — downstream can inspect partial state.
        // panicked may or may not be set depending on oxc recovery depth.
        let _ = parsed.program;
    }
}
