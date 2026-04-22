//! Per-file lint state.
//!
//! Corresponds to upstream `svelte/src/compiler/state.js` — but scoped
//! per call (one `LintContext` per file) so the whole pass is
//! rayon-parallelizable.

use std::collections::HashSet;

use smol_str::SmolStr;
use svn_core::{PositionMap, Range};

use crate::codes::Code;

/// A single emitted warning.
///
/// Byte-offset range + resolved 1-based line/col pairs. Message text
/// already has the `\nhttps://svelte.dev/e/<code>` docs-URL tail
/// appended — matches `svelte/compiler`'s output verbatim.
#[derive(Debug, Clone)]
pub struct Warning {
    pub code: Code,
    /// Rendered message (including docs URL line).
    pub message: String,
    pub range: Range,
    /// 1-based line, 1-based column (UTF-16 code units).
    pub start_line: u32,
    pub start_column: u32,
    pub end_line: u32,
    pub end_column: u32,
}

/// Whether the file opts into custom-element output via
/// `<svelte:options customElement={…}>`, parsed alongside a crude
/// inspection of the object literal's `props` key. `None` when the
/// svelte:options tag has no `customElement` attribute.
///
/// Upstream: `options.customElementOptions` parsed from the AST during
/// validate-options; we have no compile-options layer, so this is only
/// ever set by the tag. Matches `analysis.custom_element` for the
/// `options_missing_custom_element` and `custom_element_props_identifier`
/// warnings.
#[derive(Debug, Clone)]
pub struct CustomElementInfo {
    /// Whether the `customElement` option expression is an object
    /// literal with a `props` key — suppresses
    /// `custom_element_props_identifier`.
    pub has_props_option: bool,
}

/// The state threaded through every rule during a walk.
pub struct LintContext<'src> {
    pub source: &'src str,
    pub positions: PositionMap<'src>,

    /// Sink for accumulated warnings, in emit order.
    warnings: Vec<Warning>,

    /// `<!-- svelte-ignore ... -->` frames. Pushed on entering a node
    /// with leading ignore comments, popped on exit.
    ignore_stack: Vec<HashSet<SmolStr>>,

    /// True when parsing/analyzing in runes mode. Controls which rules
    /// fire (e.g. `event_directive_deprecated` only in runes mode).
    pub runes: bool,

    /// Scope tree over the instance + module scripts. Built once at
    /// the start of [`crate::walk::walk`]; rules query it by name to
    /// decide "does this identifier resolve to a local binding?" and
    /// "has it been reassigned?". `None` for the brief window before
    /// `walk()` builds it.
    pub scope_tree: Option<crate::scope::ScopeTree>,

    /// Populated from `<svelte:options customElement={…}>` when the
    /// attribute is present. Gates `custom_element_props_identifier`.
    pub custom_element_info: Option<CustomElementInfo>,

    /// Version-gated rule flags. Resolved once per batch from the
    /// user's detected `node_modules/svelte` version (see
    /// [`crate::CompatFeatures::from_version`]). Defaults to the
    /// modern superset, which matches upstream main and the
    /// `upstream_validator` fixture suite.
    pub compat: crate::compat::CompatFeatures,
}

impl<'src> LintContext<'src> {
    pub fn new(source: &'src str) -> Self {
        Self {
            source,
            positions: PositionMap::new(source),
            warnings: Vec::new(),
            ignore_stack: Vec::new(),
            runes: false,
            scope_tree: None,
            custom_element_info: None,
            compat: crate::compat::CompatFeatures::MODERN,
        }
    }

    /// Push an ignore frame. Each frame is the superset of the
    /// enclosing frame plus any new codes mentioned here — mirrors
    /// upstream `push_ignore`.
    pub fn push_ignore<I: IntoIterator<Item = SmolStr>>(&mut self, codes: I) {
        let mut next: HashSet<SmolStr> = self.ignore_stack.last().cloned().unwrap_or_default();
        next.extend(codes);
        self.ignore_stack.push(next);
    }

    pub fn pop_ignore(&mut self) {
        self.ignore_stack.pop();
    }

    /// Is `code` currently suppressed by the ignore stack?
    pub fn is_ignored(&self, code: Code) -> bool {
        self.ignore_stack
            .last()
            .is_some_and(|top| top.contains(code.as_str()))
    }

    /// Emit a warning at a range. Short-circuits if the code is
    /// currently under an ignore frame.
    pub fn emit(&mut self, code: Code, message: String, range: Range) {
        if self.is_ignored(code) {
            return;
        }
        let start = self.positions.position_of(range.start);
        let end = self.positions.position_of(range.end);
        // svelte/compiler reports 1-based line + 0-based column on
        // the Warning object but `svelte-check` itself then adds 1 to
        // column when routing to its CheckDiagnostic envelope
        // (crates/cli/src/main.rs:942 already does that). For
        // byte-parity with the bridge we match the bridge's output
        // form: 1-based line, 0-based column. CLI handles +1 column
        // at the boundary.
        self.warnings.push(Warning {
            code,
            message,
            range,
            start_line: start.line + 1,
            start_column: start.character,
            end_line: end.line + 1,
            end_column: end.character,
        });
    }

    pub fn take_warnings(self) -> Vec<Warning> {
        // Emit order: preserved as-is. Upstream's warnings array is
        // built in walker visit order (per-attribute checks fire
        // before element-level checks on the same element), and
        // fixtures compare deep-equal including order.
        self.warnings
    }
}
