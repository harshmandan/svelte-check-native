//! `SemanticModel` — aggregated analyze output for one Svelte file.
//!
//! Centralises the two pre-built analyze products (PropsInfo and
//! TemplateSummary) so consumers that need both — emit's render
//! function builder, the route-prop synth path, the lint pass — can
//! accept a single `&SemanticModel` reference instead of threading
//! each output as a separate argument.
//!
//! ## What lives here, and what doesn't
//!
//! Bundled here are concerns that emit + (future) lint would each
//! consume the same way: a Props-shape decision, a template
//! summary. Both are produced exactly once per file by their
//! respective passes (`PropsInfo::build` and `walk_template`).
//!
//! NOT bundled here: the stateful accumulator helpers
//! (`collect_top_level_bindings`, `find_store_refs_with_bindings`,
//! `collect_typed_uninit_lets`, etc.). Those are driven by emit at
//! specific points in its flow — `collect_top_level_bindings` for
//! example unions identifiers across three different parsed
//! programs (module / instance / rewritten-instance). Forcing them
//! into a build-up-front model would require either pre-computing
//! everything (wasted work for the rewritten-program path that's
//! only triggered by Svelte-4 reactive declarations) or producing
//! a lazy facade that just defers back to the same helpers. Per
//! `CLAUDE.md`'s "don't invent placeholder fields with no reader"
//! rule, a free helper joins SemanticModel only when a second
//! consumer needs the same answer.

use crate::props::PropsInfo;
use crate::template_walker::TemplateSummary;

/// Bundled analyze outputs for one component.
///
/// Construct with [`SemanticModel::new`]; both fields are owned so
/// the struct can be passed by value into emit and stay alive for
/// the duration of the emission.
#[derive(Debug, Clone, Default)]
pub struct SemanticModel {
    pub props: PropsInfo,
    pub template: TemplateSummary,
}

impl SemanticModel {
    pub fn new(props: PropsInfo, template: TemplateSummary) -> Self {
        Self { props, template }
    }
}
