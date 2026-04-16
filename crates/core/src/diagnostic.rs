//! Diagnostic type shared across all stages of the pipeline.
//!
//! A `Diagnostic` has the minimum information needed to render every output
//! format the CLI supports:
//! - `machine`:         timestamp + severity + file + 1-based line:col + message
//! - `machine-verbose`: a JSON object with start/end LSP positions
//! - `human`:           colored file:line:col + severity + message
//! - `human-verbose`:   human + 3-line source context
//!
//! Mapping from `Range` (byte offsets) to line/column happens at the
//! formatter via [`crate::PositionMap`].

use crate::range::Range;
use crate::symbol::Symbol;

/// Severity of a diagnostic. Matches LSP DiagnosticSeverity ordering.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Error,
    Warning,
    /// LSP "Information". We accept it from tsgo but `machine` output skips
    /// it (matches upstream svelte-check behavior).
    Info,
    /// LSP "Hint". Same treatment as `Info`.
    Hint,
}

impl Severity {
    /// Upper-case label used in `machine` and `machine-verbose` output.
    #[inline]
    pub fn machine_label(self) -> Option<&'static str> {
        match self {
            Self::Error => Some("ERROR"),
            Self::Warning => Some("WARNING"),
            Self::Info | Self::Hint => None,
        }
    }
}

/// Source of a diagnostic. Maps to `--diagnostic-sources` CLI filter values.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosticSource {
    /// TypeScript diagnostics from a `.ts`/`.tsx` file (or generated `.svelte.ts`).
    Ts,
    /// TypeScript diagnostics from a `.js`/`.jsx` file.
    Js,
    /// Svelte compiler warnings (syntax, unused CSS, a11y, runes).
    Svelte,
    /// CSS validation diagnostics.
    Css,
}

impl DiagnosticSource {
    /// Lowercase label used in `machine-verbose` JSON output.
    #[inline]
    pub fn label(self) -> &'static str {
        match self {
            Self::Ts => "ts",
            Self::Js => "js",
            Self::Svelte => "svelte",
            Self::Css => "css",
        }
    }

    /// Parse the textual label used in `--diagnostic-sources`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "ts" => Some(Self::Ts),
            "js" => Some(Self::Js),
            "svelte" => Some(Self::Svelte),
            "css" => Some(Self::Css),
            _ => None,
        }
    }
}

/// A diagnostic produced by any stage of the pipeline.
///
/// `code` is open-vocabulary: TS diagnostics use numeric codes (`"2322"`,
/// `"2741"`); Svelte/css/a11y use string slugs (`"css-unused-selector"`,
/// `"a11y-missing-attribute"`). Storing both as `Symbol` avoids branching in
/// output paths.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Diagnostic {
    pub range: Range,
    pub severity: Severity,
    pub source: DiagnosticSource,
    pub code: Option<Symbol>,
    pub message: String,
    /// Optional docs URL — included in `machine-verbose` JSON as `codeDescription.href`.
    pub code_description_href: Option<String>,
}

impl Diagnostic {
    pub fn error(range: Range, source: DiagnosticSource, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Error,
            source,
            code: None,
            message: message.into(),
            code_description_href: None,
        }
    }

    pub fn warning(range: Range, source: DiagnosticSource, message: impl Into<String>) -> Self {
        Self {
            range,
            severity: Severity::Warning,
            source,
            code: None,
            message: message.into(),
            code_description_href: None,
        }
    }

    #[must_use]
    pub fn with_code(mut self, code: impl Into<Symbol>) -> Self {
        self.code = Some(code.into());
        self
    }

    #[must_use]
    pub fn with_code_description(mut self, href: impl Into<String>) -> Self {
        self.code_description_href = Some(href.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_machine_labels() {
        assert_eq!(Severity::Error.machine_label(), Some("ERROR"));
        assert_eq!(Severity::Warning.machine_label(), Some("WARNING"));
        assert_eq!(Severity::Info.machine_label(), None);
        assert_eq!(Severity::Hint.machine_label(), None);
    }

    #[test]
    fn source_round_trip() {
        for s in ["ts", "js", "svelte", "css"] {
            let parsed = DiagnosticSource::parse(s).expect("known source");
            assert_eq!(parsed.label(), s);
        }
        assert_eq!(DiagnosticSource::parse("a11y"), None);
        assert_eq!(DiagnosticSource::parse(""), None);
    }

    #[test]
    fn source_serde_is_lowercase() {
        let json = serde_json::to_string(&DiagnosticSource::Svelte).expect("serialize");
        assert_eq!(json, "\"svelte\"");
        let parsed: DiagnosticSource = serde_json::from_str("\"ts\"").expect("deserialize");
        assert_eq!(parsed, DiagnosticSource::Ts);
    }

    #[test]
    fn builder_api_sets_fields() {
        let d = Diagnostic::error(Range::new(0, 5), DiagnosticSource::Ts, "nope")
            .with_code("2322")
            .with_code_description("https://example.test/ts/2322");
        assert_eq!(d.severity, Severity::Error);
        assert_eq!(d.code.as_deref(), Some("2322"));
        assert_eq!(
            d.code_description_href.as_deref(),
            Some("https://example.test/ts/2322")
        );
        assert_eq!(d.message, "nope");
    }

    #[test]
    fn warning_constructor() {
        let d = Diagnostic::warning(Range::new(3, 4), DiagnosticSource::Svelte, "hmm");
        assert_eq!(d.severity, Severity::Warning);
        assert_eq!(d.source, DiagnosticSource::Svelte);
        assert!(d.code.is_none());
    }
}
