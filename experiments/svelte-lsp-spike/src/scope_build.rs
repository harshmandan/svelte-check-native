//! Write the scope's overlays and its tsconfig, without running a compiler.
//!
//! Stage 1 got its diagnostics from `svn_typecheck::check`, which emits the
//! overlays and then spawns tsgo to check them. Once a warm tsgo is alive for
//! hover, that spawn is a second copy of the same program — the largest number
//! in the whole design. So this module does the emit half only, and the warm
//! child answers both diagnostics and hover from one program.
//!
//! Every piece is public API of the workspace crates: `CheckSession::prepare`
//! writes one overlay, `overlay::build` produces the tsconfig that lists them.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::position::FileMaps;
use crate::scope::Project;

/// What one prepared scope leaves behind for the query layer.
pub struct Scope {
    /// Per source file: the maps needed to move a position either way.
    pub maps: HashMap<PathBuf, FileMaps>,
    /// Source file → the overlay written for it.
    pub overlays: HashMap<PathBuf, PathBuf>,
    /// Everything the crate's own diagnostic mapper needs, keyed by overlay
    /// path exactly as it keys it. Built here rather than reimplemented: the
    /// mapper applies a set of upstream-parity filters that decide which
    /// diagnostics a user ever sees, and a second copy of those would drift.
    pub map_data: HashMap<PathBuf, svn_typecheck::MapData>,
    /// Directory holding the overlay tsconfig — the project root the warm tsgo
    /// is initialized against.
    pub root: PathBuf,
    /// The closure, in the order it was prepared. Doubles as the identity of
    /// this scope: a different list means tsgo needs a new program.
    pub files: Vec<PathBuf>,
}

pub fn prepare(
    project: &Project,
    closure: &[PathBuf],
    open_docs: &HashMap<PathBuf, String>,
) -> Result<Scope, String> {
    let narrow = project
        .narrow_tsconfig()
        .map_err(|e| format!("writing narrow tsconfig: {e}"))?;
    let t_session = std::time::Instant::now();
    let layout = svn_typecheck::CacheLayout::for_workspace(&project.workspace);
    let session = svn_typecheck::CheckSession::new(&project.workspace, None)
        .map_err(|e| format!("cache setup: {e}"))?;
    let session_ms = t_session.elapsed();
    let mut emit_ms = std::time::Duration::ZERO;
    let mut write_ms = std::time::Duration::ZERO;

    let mut scope = Scope {
        maps: HashMap::new(),
        overlays: HashMap::new(),
        map_data: HashMap::new(),
        root: layout.root.clone(),
        files: Vec::new(),
    };
    let mut generated: Vec<PathBuf> = Vec::new();

    for file in closure {
        let source: Arc<str> = match open_docs.get(file) {
            Some(text) => Arc::from(text.as_str()),
            None => match std::fs::read_to_string(file) {
                Ok(text) => Arc::from(text.as_str()),
                Err(_) => continue,
            },
        };
        let t_emit = std::time::Instant::now();
        let input = crate::diagnostics::build_input(file, source);
        emit_ms += t_emit.elapsed();
        let overlay = layout.generated_path_with_lang(file, input.is_ts_overlay);

        scope.maps.insert(
            file.clone(),
            FileMaps {
                line_map: input.line_map.clone(),
                token_map: input.token_map.clone(),
                overlay_line_starts: input.overlay_line_starts.clone(),
                source_line_starts: input.source_line_starts.clone(),
                is_ts: input.is_ts_overlay,
            },
        );
        scope.map_data.insert(
            overlay.clone(),
            svn_typecheck::MapData {
                line_map: input.line_map.clone(),
                token_map: input.token_map.clone(),
                overlay_line_starts: input.overlay_line_starts.clone(),
                source_line_starts: input.source_line_starts.clone(),
                // `prepare` writes the overlay in emit space, so the mapper can
                // read it back on demand instead of holding every overlay.
                overlay_text: svn_typecheck::LazyText::on_disk(overlay.clone()),
                source_text: input.source.clone(),
                identity_map: false,
                svelte_script_is_ts: input.is_ts_overlay,
                kit_col_shifts: Vec::new(),
                ignore_regions: svn_typecheck::scan_ignore_regions(&input.generated_ts),
                pug_template_ranges: svn_typecheck::scan_pug_template_ranges(&input.source),
            },
        );
        scope.overlays.insert(file.clone(), overlay.clone());
        scope.files.push(file.clone());
        generated.push(overlay);

        let t_write = std::time::Instant::now();
        session
            .prepare(input)
            .map_err(|e| format!("writing overlay for {}: {e}", file.display()))?;
        write_ms += t_write.elapsed();
    }

    // The kit types mirror is populated by the session in the background; pass
    // it through only if it actually materialised, exactly as `finish` does.
    let mirror = layout.kit_types_mirror_dir();
    let mirror = mirror.is_dir().then_some(mirror);

    let t_tsconfig = std::time::Instant::now();
    let tsconfig = svn_typecheck::overlay::build(
        &layout,
        &narrow,
        &generated,
        &[],
        mirror.as_deref(),
    );
    let text = serde_json::to_string_pretty(&tsconfig)
        .map_err(|e| format!("serializing overlay tsconfig: {e}"))?;
    std::fs::write(layout.root.join("tsconfig.json"), text)
        .map_err(|e| format!("writing overlay tsconfig: {e}"))?;

    crate::rpc::log(&format!(
        "  prepare: session {session_ms:?}, emit {emit_ms:?}, overlay-write {write_ms:?}, tsconfig {:?}",
        t_tsconfig.elapsed()
    ));
    Ok(scope)
}

/// Whether two scopes contain the same files. A changed file list means the
/// tsconfig changed, and tsgo has to build a new program for it.
pub fn same_files(a: &[PathBuf], b: &[PathBuf]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| x == y)
}

/// Convert one of tsgo's LSP diagnostics into the shape the crate's mapper
/// consumes. LSP counts lines and characters from zero; tsgo's own output —
/// which the mapper was written against — counts from one.
pub fn to_raw(raw: &serde_json::Value, overlay: &Path) -> Option<svn_typecheck::RawDiagnostic> {
    let start_line = raw.pointer("/range/start/line")?.as_u64()? as u32;
    let start_char = raw.pointer("/range/start/character")?.as_u64()? as u32;
    let end_line = raw.pointer("/range/end/line")?.as_u64()? as u32;
    let end_char = raw.pointer("/range/end/character")?.as_u64()? as u32;

    Some(svn_typecheck::RawDiagnostic {
        file: overlay.to_path_buf(),
        line: start_line + 1,
        column: start_char + 1,
        severity: match raw.get("severity").and_then(|s| s.as_u64()) {
            Some(1) => svn_typecheck::Severity::Error,
            Some(2) => svn_typecheck::Severity::Warning,
            _ => svn_typecheck::Severity::Hint,
        },
        code: raw.get("code").and_then(|c| c.as_u64()).unwrap_or(0) as u32,
        message: raw
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or_default()
            .to_string(),
        // Only single-line spans have a length the mapper can use; a span
        // crossing lines is left unmeasured, as it is in tsgo's own output.
        span_length: (end_line == start_line).then(|| end_char.saturating_sub(start_char)),
    })
}
