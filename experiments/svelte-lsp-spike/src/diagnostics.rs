//! Diagnostics for one scope, answered by the warm tsgo.
//!
//! Two sources, merged: TypeScript diagnostics pulled from the language server
//! that is already holding the program, and Svelte compiler warnings from a
//! per-file pass that needs no compiler at all.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::scope::Project;
use crate::scope_build::{self, Scope};
use crate::tsgo::Tsgo;

/// Emit the scope, make sure a warm tsgo is holding it, and collect every
/// diagnostic for every file in it.
pub fn check_scope(
    project: &Project,
    closure: &[PathBuf],
    open_docs: &HashMap<PathBuf, String>,
    warm: &mut Option<Tsgo>,
    last_scope: &mut Vec<PathBuf>,
    include_hints: bool,
) -> Result<(Vec<svn_typecheck::CheckDiagnostic>, Scope), String> {
    let prepare_started = std::time::Instant::now();
    let scope = scope_build::prepare(project, closure, open_docs)?;
    let prepare_ms = prepare_started.elapsed();

    // A changed file list means a changed tsconfig, and tsgo has to be told.
    // Telling it via `workspace/didChangeWatchedFiles` does not work: the
    // process keeps its old program, the newly listed overlay becomes an
    // orphan in an inferred project with no shims, and the file fills with
    // `Cannot find name '__svn_any'`. Restarting is both correct and, on a
    // narrow scope, cheaper than the notification was supposed to be — 285 ms
    // against 322 ms for a second tab. `SPIKE_KEEP_ON_SCOPE=1` keeps the old
    // path for anyone wanting to retest it against a future tsgo.
    let scope_changed = !scope_build::same_files(last_scope, &scope.files);
    let keep_on_change = std::env::var("SPIKE_KEEP_ON_SCOPE").is_ok();
    if scope_changed && !keep_on_change {
        *warm = None;
    }
    let cold = warm.is_none();
    if cold {
        let binary = svn_typecheck::discover(&project.workspace)
            .map_err(|e| format!("no TypeScript compiler: {e}"))?;
        *warm = Some(Tsgo::start(&binary.path, binary.needs_node, &scope.root)?);
    }
    *last_scope = scope.files.clone();
    let child = warm.as_mut().ok_or("tsgo unavailable")?;

    // Opening a tab rewrites the project's file list. Telling tsgo the config
    // changed lets it fold the new files into the program it already has;
    // killing the process instead would rebuild from nothing and cost about a
    // second of dead editor on every tab.
    if scope_changed && !cold && keep_on_change {
        child.config_changed(&scope.root.join("tsconfig.json"));
    }

    // Sync every overlay BEFORE asking about any of them. Interleaving the two
    // means each pull runs against a program the next didChange immediately
    // invalidates, so a 35-file scope pays 35 full re-checks instead of one.
    for file in &scope.files {
        if let Some(overlay) = scope.overlays.get(file) {
            child.sync(overlay)?;
        }
    }

    // Collect every overlay's raw diagnostics, then map them all through the
    // crate's own mapper — the same code path `check` uses, so positions and
    // the upstream-parity filters are shared rather than duplicated.
    let mut raws = Vec::new();
    for file in &scope.files {
        let Some(overlay) = scope.overlays.get(file) else { continue };
        // A JavaScript component is only type-checked when the project asks
        // for it, or when the file opts in with `// @ts-check`. tsgo's LSP
        // hands back semantic diagnostics for open `.js` documents either way.
        let js_overlay = scope.maps.get(file).map(|m| !m.is_ts).unwrap_or(false);
        if js_overlay && !project.check_js && !opts_into_checking(file, open_docs) {
            continue;
        }
        for raw in child.diagnostics(overlay)? {
            if let Some(converted) = scope_build::to_raw(&raw, overlay) {
                raws.push(converted);
            }
        }
    }
    let layout = svn_typecheck::CacheLayout::for_workspace(&project.workspace);
    let svelte5_plus = svn_typecheck::workspace_svelte_is_5_plus(&project.workspace);
    let mut diags: Vec<svn_typecheck::CheckDiagnostic> =
        svn_typecheck::map_raw_diagnostics(&layout, &scope.map_data, raws, svelte5_plus)
            .into_iter()
            // Hint-severity diagnostics — unused locals, deprecations — split
            // the two surfaces apart. svelte-check's CLI writers drop them; its
            // language server always asks for them (DiagnosticsProvider.ts
            // calls getSuggestionDiagnostics beside getSemanticDiagnostics),
            // which is why upstream's LS fixtures expect TS6133 and friends. So
            // an editor gets them and the CLI-parity sweep does not.
            .filter(|d| include_hints || !matches!(d.severity, svn_typecheck::Severity::Hint))
            .collect();

    // Svelte compiler warnings: a per-file pass with no tsgo in it, and the
    // majority of what a user sees day to day.
    let compat = svn_lint::detect_for_workspace(&project.workspace);
    for file in &scope.files {
        let source = match open_docs.get(file) {
            Some(text) => text.clone(),
            None => match std::fs::read_to_string(file) {
                Ok(text) => text,
                Err(_) => continue,
            },
        };
        for w in svn_lint::lint_file(&source, file, None, compat) {
            diags.push(svn_typecheck::CheckDiagnostic {
                source_path: file.clone(),
                line: w.start_line,
                // Lint columns are 0-based; CheckDiagnostic is 1-based.
                column: w.start_column + 1,
                end_line: w.end_line,
                end_column: w.end_column + 1,
                severity: svn_typecheck::Severity::Warning,
                code: svn_typecheck::DiagnosticCode::Slug(w.code.as_str().to_string()),
                message: w.message,
                source: svn_typecheck::DiagnosticSource::Svelte,
                code_description_url: None,
            });
        }
    }

    crate::rpc::log(&format!(
        "  phases: emit+write {:?}, tsgo {:?}",
        prepare_ms,
        prepare_started.elapsed() - prepare_ms
    ));
    Ok((diags, scope))
}

/// Parse, walk, emit — the same sequence `crates/cli/src/main.rs` runs for
/// every file it checks.
pub fn build_input(file: &Path, source: Arc<str>) -> svn_typecheck::CheckInput {
    let (doc, _parse_errors) = svn_parser::parse_sections(&source);
    let (fragment, _template_errors) =
        svn_parser::parse_all_template_runs(&source, &doc.template.text_runs);
    let summary = svn_analyze::walk_template(&fragment, &source);
    let is_ts = doc.script_lang() == svn_parser::ScriptLang::Ts;
    let emitted = svn_emit::emit_document_with_lang(&doc, &fragment, &summary, file, is_ts);

    svn_typecheck::CheckInput {
        source_path: file.to_path_buf(),
        source,
        generated_ts: emitted.typescript,
        line_map: emitted.line_map,
        token_map: emitted.token_map,
        overlay_line_starts: emitted.overlay_line_starts,
        source_line_starts: emitted.source_line_starts,
        kit_col_shifts: Vec::new(),
        kind: svn_typecheck::InputKind::Svelte,
        is_ts_overlay: is_ts,
    }
}

/// Does this component ask to be type-checked despite having a plain
/// `<script>`? Matches the emit crate's own permissive reading of the
/// directive: a line or block comment anywhere in the file.
fn opts_into_checking(file: &Path, open_docs: &HashMap<PathBuf, String>) -> bool {
    let text = match open_docs.get(file) {
        Some(text) => text.clone(),
        None => std::fs::read_to_string(file).unwrap_or_default(),
    };
    text.contains("@ts-check")
}
