//! Components svelte2tsx can't convert.
//!
//! `svelte-check --tsgo` converts each component with svelte2tsx and
//! deletes the manifest entry of one that throws, so its Svelte pass,
//! which visits only manifest entries, never sees it: the component
//! checks clean, isn't counted, and only the build fails. The default
//! engine reports two errors for such a component, both computed here
//! by the project's own `svelte/compiler`:
//!
//! - its Svelte pass compiles the component with the Svelte config's
//!   `compilerOptions` exactly as given and reports what the compiler
//!   throws (source `svelte`, the compiler's code). The language server
//!   then treats it the way `createParserErrorDiagnostic` does: a
//!   message containing "expected" is dropped when the project has no
//!   `preprocess` of its own and a script or style tag carries a
//!   `lang`/`type` attribute (it is taken to be syntax a preprocessor
//!   would have handled), and otherwise gets a preprocessing hint when
//!   it lies inside the script or style tag;
//! - its TypeScript pass reports svelte2tsx's own error unchanged
//!   (source `ts` or `js` by the script's language, code `-1`).

use std::path::{Path, PathBuf};

use svn_core::{Position, PositionMap};
use svn_parser::{Document, ScriptAttr};
use svn_svelte_compiler::CompilerDiagnostic;

const HINT_INTRO: &str = "\n\nIf you expect this syntax to work, here are some suggestions: ";
const SCRIPT_HINT: &str = "\nIf you use typescript with `svelte-preprocess`, did you add `lang=\"ts\"` to your `script` tag? ";
const STYLE_HINT: &str = "\nIf you use less/SCSS with `svelte-preprocess`, did you add `lang=\"scss\"`/`lang=\"less\"` to your `style` tag? If you use SCSS, it may be necessary to add the path to your NODE runtime to the setting `svelte.language-server.runtime`, or use `sass` instead of `node-sass`. ";
const HINT_OUTRO: &str = "\nDid you setup a `svelte.config.js` or `vite.config.js`? \nSee https://github.com/sveltejs/language-tools/tree/master/docs#using-with-preprocessors for more info.";

/// A component svelte2tsx can't convert, with what svelte-check's
/// default engine reports for it (in its order: Svelte pass first).
pub struct ParseRejection {
    pub file: PathBuf,
    pub diagnostics: Vec<RejectionDiagnostic>,
}

/// One error for a rejected component. Positions are 1-based (line and
/// UTF-16 column), matching the CLI's `CheckDiagnostic`.
pub struct RejectionDiagnostic {
    pub line: u32,
    pub column: u32,
    pub end_line: u32,
    pub end_column: u32,
    pub kind: RejectionKind,
    pub message: String,
}

pub enum RejectionKind {
    /// The compiler's error from the Svelte pass, with its code.
    Compiler(String),
    /// svelte2tsx's error from the TypeScript pass; `ts` says whether the
    /// component's script is TypeScript (source `ts`, else `js`).
    Conversion { ts: bool },
}

/// Ask the compiler about `files` (components our parser expects
/// svelte2tsx to reject) and return the ones svelte2tsx does reject.
/// `preprocess_configured` says whether the Svelte config governing a
/// file supplies its own `preprocess`. `None` when the compiler couldn't
/// be run: the caller then keeps `--tsgo`'s behaviour.
pub fn parse_rejections(
    workspace: &Path,
    files: &[(PathBuf, String)],
    preprocess_configured: impl Fn(&Path) -> bool,
) -> Option<Vec<ParseRejection>> {
    let answers = svn_svelte_compiler::rejections(workspace, files).ok()?;
    let rejections = answers
        .into_iter()
        .filter_map(|(file, answer)| {
            let parse_error = answer.parse_error?;
            let source = files.iter().find(|(f, _)| *f == file).map(|(_, s)| s)?;
            let (doc, _) = svn_parser::parse_sections(source);
            let mut diagnostics = Vec::with_capacity(2);
            if let Some(error) = answer.compile_error {
                diagnostics.extend(svelte_pass_diagnostic(
                    source,
                    &doc,
                    error,
                    preprocess_configured(&file),
                ));
            }
            let ts = script_is_ts(&doc);
            diagnostics.push(diagnostic(
                parse_error.start,
                parse_error.end,
                RejectionKind::Conversion { ts },
                parse_error.message,
            ));
            Some(ParseRejection { file, diagnostics })
        })
        .collect();
    Some(rejections)
}

fn svelte_pass_diagnostic(
    source: &str,
    doc: &Document<'_>,
    error: CompilerDiagnostic,
    preprocess_configured: bool,
) -> Option<RejectionDiagnostic> {
    let mut message = error.message;
    if message.contains("expected") {
        if !preprocess_configured && has_language_attribute(doc) {
            return None;
        }
        // The compiler's lines are 1-based, its columns 0-based UTF-16.
        let at = Position {
            line: error.start.line.saturating_sub(1),
            character: error.start.column,
        };
        let map = PositionMap::new(source);
        let within = |range: svn_core::Range| {
            let (from, to) = map.range_positions(range);
            (from.line, from.character) <= (at.line, at.character)
                && (at.line, at.character) <= (to.line, to.character)
        };
        let in_script = doc
            .instance_script
            .as_ref()
            .or(doc.module_script.as_ref())
            .is_some_and(|s| within(s.content_range));
        let in_style = doc.style.as_ref().is_some_and(|s| within(s.content_range));
        if in_style || in_script {
            message.push_str(HINT_INTRO);
            message.push_str(if in_script { SCRIPT_HINT } else { STYLE_HINT });
            message.push_str(HINT_OUTRO);
        }
    }
    Some(diagnostic(
        error.start,
        error.end,
        RejectionKind::Compiler(error.code),
        message,
    ))
}

fn diagnostic(
    start: svn_svelte_compiler::Position,
    end: svn_svelte_compiler::Position,
    kind: RejectionKind,
    message: String,
) -> RejectionDiagnostic {
    RejectionDiagnostic {
        line: start.line,
        column: start.column.saturating_add(1),
        end_line: end.line,
        end_column: end.column.saturating_add(1),
        kind,
        message,
    }
}

/// The last value of a `lang` or `type` attribute, as the language
/// server's attribute map keeps it.
fn language(attrs: &[ScriptAttr]) -> Option<&str> {
    let value = |name: &str| {
        attrs
            .iter()
            .rev()
            .find(|a| a.name == name)
            .and_then(|a| a.value.as_deref())
            .filter(|v| !v.is_empty())
    };
    value("lang").or_else(|| value("type"))
}

/// svelte-check's `hasLanguageAttribute`: a non-empty `lang` or `type`
/// (`text/` prefix ignored) on the instance script's tag (the module
/// script's when there is none) or on the style tag.
fn has_language_attribute(doc: &Document<'_>) -> bool {
    let set = |attrs: &[ScriptAttr]| {
        language(attrs).is_some_and(|v| !v.strip_prefix("text/").unwrap_or(v).is_empty())
    };
    doc.instance_script
        .as_ref()
        .or(doc.module_script.as_ref())
        .is_some_and(|s| set(&s.attrs))
        || doc.style.as_ref().is_some_and(|s| set(&s.attrs))
}

/// The language server's script kind for the snapshot: TypeScript when
/// either script's `lang`/`type` names TypeScript.
fn script_is_ts(doc: &Document<'_>) -> bool {
    [doc.instance_script.as_ref(), doc.module_script.as_ref()]
        .into_iter()
        .flatten()
        .any(|s| {
            matches!(
                language(&s.attrs),
                Some("ts" | "typescript" | "text/ts" | "text/typescript")
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use svn_svelte_compiler::{Position as At, Severity};

    fn error(message: &str, line: u32, column: u32) -> CompilerDiagnostic {
        CompilerDiagnostic {
            code: "js_parse_error".to_string(),
            message: message.to_string(),
            severity: Severity::Error,
            start: At { line, column },
            end: At { line, column },
        }
    }

    fn svelte_pass(
        source: &str,
        e: CompilerDiagnostic,
        configured: bool,
    ) -> Option<RejectionDiagnostic> {
        let (doc, _) = svn_parser::parse_sections(source);
        svelte_pass_diagnostic(source, &doc, e, configured)
    }

    #[test]
    fn expected_message_is_dropped_without_preprocess_when_a_tag_has_lang() {
        let source = "<script lang=\"ts\">\n</script>\n{#if a}<div>{/if}\n";
        let e = error("Unexpected block closing tag", 3, 12);
        assert!(svelte_pass(source, e, false).is_none());
    }

    #[test]
    fn expected_message_is_kept_with_a_configured_preprocess() {
        let source = "<script lang=\"ts\">\n</script>\n{#if a}<div>{/if}\n";
        let e = error("Unexpected block closing tag", 3, 12);
        let d = svelte_pass(source, e, true).unwrap();
        assert_eq!((d.line, d.column), (3, 13));
        assert_eq!(d.message, "Unexpected block closing tag");
    }

    #[test]
    fn expected_message_in_the_script_gets_the_hint() {
        let source = "<script>\n  let a = ;\n</script>\n";
        let e = error("Unexpected token", 2, 10);
        let d = svelte_pass(source, e, false).unwrap();
        assert!(d.message.starts_with("Unexpected token\n\nIf you expect"));
        assert!(d.message.contains("`script` tag"));
    }

    #[test]
    fn expected_message_in_the_style_gets_the_style_hint() {
        let source = "<style>\n  p { color: red\n</style>\n";
        let e = error(
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            2,
            16,
        );
        let d = svelte_pass(source, e, false).unwrap();
        assert!(d.message.contains("`style` tag"));
    }

    #[test]
    fn other_messages_pass_through() {
        let source = "<script lang=\"ts\">\n</script>\n<div>\n";
        let e = error("`<div>` was left open", 3, 0);
        let d = svelte_pass(source, e, false).unwrap();
        assert_eq!(d.message, "`<div>` was left open");
        assert_eq!((d.line, d.column, d.end_line, d.end_column), (3, 1, 3, 1));
    }

    #[test]
    fn script_kind_follows_either_script_lang() {
        let ts = |src: &str| script_is_ts(&svn_parser::parse_sections(src).0);
        assert!(ts("<script lang=\"ts\"></script>"));
        assert!(ts(
            "<script module type=\"text/typescript\"></script><script></script>"
        ));
        assert!(!ts("<script></script>"));
        assert!(!ts("<script lang=\"js\"></script>"));
    }
}
