//! tsgo integration + tsconfig overlay.
//!
//! ### Pipeline
//!
//! 1. The CLI hands us a list of `.svelte` source files plus the matching
//!    emitted TypeScript (one string per source).
//! 2. We write each generated `.svelte.ts` into the cache directory using
//!    [`cache::write_if_changed`] (so unchanged files don't perturb tsgo's
//!    incremental build info).
//! 3. We write a `.d.svelte.ts` re-export stub for each component so
//!    `import Foo from './Foo.svelte'` resolves at the type level.
//! 4. We generate an overlay tsconfig that extends the user's tsconfig
//!    and lists every generated file via `files`, plus the tsgo-mandatory
//!    `allowArbitraryExtensions: true` and `noEmit: true`.
//! 5. We spawn tsgo with `--project <overlay>.json --pretty true
//!    --noErrorTruncation`, capture combined stdout+stderr, and parse with
//!    [`output::parse`].
//! 6. We map diagnostics back to original `.svelte` paths via the cache
//!    layout. Line/column mapping is best-effort for now (we account for
//!    the wrapper offset added by [`emit::emit_document`] but don't yet
//!    have a real source map).
//!
//! ### What's not yet here
//!
//! - Proper source-map mapping (currently we apply a fixed offset matching
//!   the emitter's wrapper; once the emitter writes sourcemap mappings,
//!   this becomes a v3 source-map consumer).
//! - Path-aliased tsconfig support (`paths`/`rootDirs`/`extends-array`
//!   beyond what the user's tsconfig handles via the extends chain itself).
//! - `references` (project references) propagation.

#![cfg_attr(test, allow(clippy::expect_used, clippy::unwrap_used))]

pub mod cache;
pub mod discovery;
mod filters;
pub mod kit_app_ambients;
pub mod kit_types_mirror;
pub mod output;
pub mod overlay;
mod path_utils;
mod position;
pub mod runner;
mod types;

use std::path::{Path, PathBuf};

pub use cache::{CacheLayout, write_if_changed};
pub use discovery::{DiscoveryError, TsgoBinary, discover};
pub use filters::scan_ignore_regions;
pub use output::{RawDiagnostic, Severity, parse as parse_output};
pub use runner::{RunError, run as run_tsgo};
pub use types::{
    CheckDiagnostic, CheckError, CheckInput, CheckOutput, DiagnosticCode, DiagnosticSource,
    IGNORE_END_MARKER, IGNORE_START_MARKER, InputKind, MapData,
};

/// Svelte type shims — single source of truth for everything we ship
/// into the cache. Structure:
///
/// - Always-shipped core: rune ambients (`$state`, `$derived`, …),
///   emit helper types (`__SvnStoreValue`, `__svn_type_ref`), the
///   `svelteHTML` / `svelte-jsx` intersection shapes, asset-module
///   ambients (`*.svg` / `*.css` / …).
/// - `@@FALLBACK_BEGIN@@` … `@@FALLBACK_END@@` block: stand-in
///   `declare module 'svelte/*'` declarations for environments
///   without a real `svelte` install. Stripped before writing to the
///   cache when a real `svelte` IS reachable — otherwise the fallback
///   would shadow the richer real types (`HTMLAnchorAttributes` from
///   `svelte/elements` etc.) and fire false-positive TS2305 errors.
const SVELTE_SHIMS: &str = include_str!("svelte_shims_core.d.ts");

const FALLBACK_BEGIN: &str = "// @@FALLBACK_BEGIN@@";
const FALLBACK_END: &str = "// @@FALLBACK_END@@";
const STATE_AMBIENTS_BEGIN: &str = "// @@STATE_AMBIENTS_BEGIN@@";
const STATE_AMBIENTS_END: &str = "// @@STATE_AMBIENTS_END@@";

/// Marker pair used by emit to wrap emit-synthesised scaffolding
/// bytes that should never produce user-visible diagnostics. Mirrors
/// upstream svelte2tsx's `/*Ωignore_startΩ*/…/*Ωignore_endΩ*/` pair
/// (see `language-tools/packages/language-server/src/plugins/
/// typescript/features/utils.ts:86-109`) but uses an ASCII-only
/// spelling so byte-offset arithmetic stays simple.
///
/// The filter lives in [`map_diagnostic`]: when a diagnostic's
/// overlay byte offset falls between a `IGNORE_START_MARKER` and
/// the next matching `IGNORE_END_MARKER`, the diagnostic is
/// dropped. Lets emit mark regions like component-instantiation
/// scaffolding (`$$bindings = '...'` trail, intermediate helper
/// locals) as "this is our code, not the user's — don't surface
/// errors here".
/// Return the shim text with the fallback `declare module 'svelte/*'`
/// block AND our `$state<T>` ambient overloads stripped when
/// `keep_fallback` is false (i.e. real svelte is installed). Line
/// count is preserved — stripped ranges are replaced with blank lines
/// so diagnostic positions in the shim stay stable.
///
/// Why strip `$state<T>`: Svelte 5's `types/index.d.ts:3221-3222`
/// declares the same two overloads. Keeping both produces 4 identical
/// overloads, which poisons TS's overload resolution — a mismatch
/// reports TS2769 "No overload matches this call" instead of the
/// expected TS2741 on structurally-incomplete initial values. Other
/// rune ambients ($derived/$effect/$props/etc.) aren't stripped —
/// either single-overload forms don't hit the dedup issue or our
/// shim carries extra overloads (e.g. `$props<T>()`) that Svelte's
/// simpler `$props(): any` doesn't provide.
fn resolve_shim_text(keep_fallback: bool) -> String {
    if keep_fallback {
        return SVELTE_SHIMS.to_string();
    }
    let mut out = SVELTE_SHIMS.to_string();
    out = strip_range_blanking(&out, FALLBACK_BEGIN, FALLBACK_END);
    out = strip_range_blanking(&out, STATE_AMBIENTS_BEGIN, STATE_AMBIENTS_END);
    out
}

/// Replace the text between `begin` and `end` markers (inclusive)
/// with blank lines, preserving line count so diagnostic positions in
/// the shim stay stable.
fn strip_range_blanking(text: &str, begin_marker: &str, end_marker: &str) -> String {
    let Some(begin) = text.find(begin_marker) else {
        return text.to_string();
    };
    let Some(end_offset) = text[begin..].find(end_marker) else {
        return text.to_string();
    };
    let end = begin + end_offset + end_marker.len();
    let stripped = &text[begin..end];
    let mut out = String::with_capacity(text.len());
    out.push_str(&text[..begin]);
    for _ in 0..stripped.bytes().filter(|&b| b == b'\n').count() {
        out.push('\n');
    }
    out.push_str(&text[end..]);
    out
}

/// Walk up from `workspace` looking for `node_modules/svelte/package.json`.
/// Returns `true` iff the user has the real `svelte` package installed
/// somewhere in the resolution chain.
fn has_real_svelte(workspace: &Path) -> bool {
    svn_core::walk_up_dirs(workspace, |dir| {
        dir.join(svn_core::NODE_MODULES_DIR)
            .join("svelte")
            .join("package.json")
            .is_file()
            .then_some(())
    })
    .is_some()
}

pub use svn_emit::{LineMapEntry, TokenMapEntry};

/// Walk `svelte_dir` and delete every regular file not present in
/// `written_paths`. Used as the cache GC step after the per-input
/// write loop in [`check`]. Errors during traversal or deletion are
/// swallowed — a stale orphan is recoverable next run, and we
/// shouldn't fail a type-check over a transient filesystem issue
/// (Windows file locks, antivirus, NFS lag).
fn gc_orphaned_overlays(svelte_dir: &Path, written_paths: &std::collections::HashSet<PathBuf>) {
    if !svelte_dir.is_dir() {
        return;
    }
    for entry in walkdir::WalkDir::new(svelte_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|e| e.file_type().is_file())
    {
        let path = entry.path();
        if !written_paths.contains(path) {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Run the full type-check pipeline.
///
/// `workspace` is the user's project root; `user_tsconfig` is the absolute
/// path to their `tsconfig.json` (or `jsconfig.json`); `inputs` is the list
/// of files-to-check with their generated TS.
///
/// Consumes `inputs` so each `generated_ts` string drops as soon as it has
/// been written to the cache — the bridge phase that runs after this call
/// can hold ~100 MB of bun heaps; we don't also want to keep a duplicate
/// of every overlay TS in our own RSS waiting to be GC'd at end-of-fn.
///
/// Returns mapped diagnostics + the count of files in tsgo's program
/// (used for the COMPLETED line's `<N> FILES` denominator so it
/// matches upstream svelte-check's number — upstream prints the
/// LanguageService program count, not just the `.svelte` walker
/// count). On success with no problems the diagnostics vec is empty.
pub fn check(
    workspace: &Path,
    solution_root_tsconfig: Option<&Path>,
    user_tsconfig: &Path,
    inputs: Vec<CheckInput>,
    extended_diagnostics: bool,
) -> Result<CheckOutput, CheckError> {
    let layout = CacheLayout::for_workspace_with_solution_root(
        workspace,
        solution_root_tsconfig.map(|p| p.to_path_buf()),
    );
    std::fs::create_dir_all(&layout.svelte_dir)?;

    // Ship the svelte type shims into the cache. Core (runes + helper
    // types + shim-wide ambients) is always written. The fallback
    // `declare module 'svelte/*'` block — marked in-source with
    // @@FALLBACK_BEGIN@@ / @@FALLBACK_END@@ — is stripped when a real
    // svelte install is reachable; otherwise its minimal declarations
    // would shadow the richer real types (e.g. svelte/elements
    // re-exports HTMLAnchorAttributes, HTMLInputAttributes, ClassValue
    // from clsx, etc., none of which the fallback enumerates).
    let shim_text = resolve_shim_text(/* keep_fallback */ !has_real_svelte(workspace));
    write_if_changed(&layout.svelte_shims, &shim_text)?;

    // Step 1: write generated TS for each input. Skip identical writes.
    // Collect a per-overlay-path line map so the diagnostic mapper can
    // translate tsgo's overlay positions back to source positions.
    //
    // Note: no separate `.d.svelte.ts` re-export stub is written. The
    // generated `<name>.svelte.ts` IS the type definition for
    // `<name>.svelte` — emit rewrites every `import './X.svelte'` to
    // `import './X.svelte.js'` so TS's standard `.js`→`.ts` resolver
    // lands on our generated file rather than the `*.svelte` ambient
    // declaration that the `svelte` package ships.
    let mut generated_paths: Vec<PathBuf> = Vec::with_capacity(inputs.len());
    let mut kit_overlay_sources: Vec<PathBuf> = Vec::new();
    let mut map_data: std::collections::HashMap<PathBuf, MapData> =
        std::collections::HashMap::with_capacity(inputs.len());
    // Track every cache file we touch this run. Anything in
    // `svelte_dir` not in this set after the loop is orphaned (the
    // source `.svelte` was deleted or renamed) and gets garbage-
    // collected so stale overlays / ambients don't keep masking real
    // "module not found" errors at later imports.
    let mut written_paths: std::collections::HashSet<PathBuf> =
        std::collections::HashSet::with_capacity(inputs.len() * 2);
    // `inputs` is consumed here — `generated_ts` and `line_map` move out
    // of each `CheckInput` and the string drops at end of iteration.
    for mut input in inputs {
        let gen_path = match input.kind {
            InputKind::Svelte | InputKind::SvelteAuxiliary => {
                layout.generated_path_with_lang(&input.source_path, input.is_ts_overlay)
            }
            InputKind::KitFile | InputKind::UserTsOverlay => {
                layout.kit_overlay_path(&input.source_path)
            }
        };
        // When the source's script-lang toggles between JS and TS
        // across runs, the previously-written sibling (`.svn.ts` when
        // we now emit `.svn.js`, or vice versa) becomes stale. TS's
        // bundler resolver prefers `.ts` when `./foo.svelte.svn.js` is
        // imported, so a stale `.svn.ts` wins and tsgo reads outdated
        // emit. Remove the other-extension sibling on every write —
        // cheap `fs::remove_file` ignored-not-found.
        if matches!(input.kind, InputKind::Svelte | InputKind::SvelteAuxiliary) {
            let sibling = layout.generated_path_with_lang(&input.source_path, !input.is_ts_overlay);
            let _ = std::fs::remove_file(&sibling);
        }
        // Rewrite `../`-starting imports that point outside the
        // workspace so they resolve from the OVERLAY's location, not
        // the source's. Mirrors upstream svelte2tsx's
        // `helpers/rewriteExternalImports.ts`. The overlay lives
        // under `<workspace>/node_modules/.cache/svelte-check-native/
        // svelte/...` whereas the source lives at
        // `<workspace>/src/...`; a Svelte component that imports a
        // sibling-package file via `../../../sibling/src/lib/foo.ts`
        // (Threlte/xr → Threlte/extras pattern) would resolve from
        // the SOURCE dir but fail from the overlay dir, since the
        // path traversal stops inside the cache. The rewrite changes
        // the specifier string only — line/column counts unchanged
        // so source-map data stays valid.
        if matches!(input.kind, InputKind::Svelte | InputKind::SvelteAuxiliary) {
            input.generated_ts = path_utils::rewrite_external_imports(
                &input.generated_ts,
                &input.source_path,
                &gen_path,
                &layout.workspace,
            );
        }
        write_if_changed(&gen_path, &input.generated_ts)?;
        written_paths.insert(gen_path.clone());

        match input.kind {
            InputKind::Svelte | InputKind::SvelteAuxiliary => {
                // Ambient `.d.svelte.ts` sidecar next to the overlay.
                // Makes `import './Foo.svelte'` in user-controlled files
                // (barrel re-exports, plain `.ts` modules we don't
                // rewrite, Svelte files OUTSIDE the tsconfig scope that
                // are imported by in-scope ones) resolve to the
                // overlay's types via TS's `allowArbitraryExtensions`
                // mechanism. Content is a one-shot re-export from the
                // overlay; no diagnostic should ever fire on this file.
                //
                // Written for SvelteAuxiliary inputs too — those exist
                // PRECISELY so tsgo's import-following can reach an
                // out-of-scope Svelte file and pick up our overlay's
                // types.
                //
                // KNOWN LIMITATION: this ambient doesn't help in the
                // sibling-collision case where the user has both
                // `Foo.svelte` AND `Foo.svelte.ts` (a Svelte 5 runes
                // module) in the same directory. tsgo's resolver picks
                // the workspace as the `rootDirs` match for
                // `./Foo.svelte` (because the physical file lives
                // there, not in cache) and searches WITHIN the
                // workspace — so it finds the runes module via bundler
                // auto-extension (`.svelte` → `.svelte.ts`) before our
                // cache-resident ambient is tried. Writing a
                // `.d.svelte.ts` into the cache at the mirrored path is
                // unreachable for this specific import. Observed on
                // shadcn-svelte-style barrel `index.ts` re-exports in
                // the wild. Real fix would require either writing
                // ambients into the user's source tree (invasive) or
                // pre-rewriting every user-owned `.ts` file that
                // imports `.svelte` (high scope). Deferred.
                let ambient_path = layout.ambient_path(&input.source_path);
                let overlay_file_name = gen_path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown.svelte.svn.ts");
                let ambient_text = format!(
                    "// generated by svelte-check-native; do not edit\n\
                     export {{ default }} from './{overlay_file_name}';\n\
                     export * from './{overlay_file_name}';\n"
                );
                write_if_changed(&ambient_path, &ambient_text)?;
                written_paths.insert(ambient_path);
            }
            InputKind::KitFile | InputKind::UserTsOverlay => {
                // Mirror-overlay kinds: original source path goes into
                // the overlay tsconfig's `exclude` so tsgo reads only
                // our rewritten version. KitFile carries injected
                // route / hooks types; UserTsOverlay carries rewritten
                // `.svelte` imports that bypass the sibling-runes-module
                // collision.
                kit_overlay_sources.push(input.source_path.clone());
            }
        }

        let ignore_regions = scan_ignore_regions(&input.generated_ts);
        // Source text for the position mapper. For Svelte / Aux
        // overlays the source is the user's `.svelte` file; for kit /
        // user-ts overlays the original layout is preserved through
        // the inject and rewrite paths so we can reuse the overlay
        // text as the source view (identity_map=true on those kinds
        // already handles the line/col pass-through, but having the
        // text on hand keeps position_to_byte / byte_to_position's
        // UTF-16 conversion correct on non-ASCII content).
        let source_text = match input.kind {
            InputKind::Svelte | InputKind::SvelteAuxiliary => {
                std::fs::read_to_string(&input.source_path).unwrap_or_default()
            }
            InputKind::KitFile | InputKind::UserTsOverlay => input.generated_ts.clone(),
        };
        let overlay_text = std::mem::take(&mut input.generated_ts);
        map_data.insert(
            gen_path.clone(),
            MapData {
                line_map: input.line_map,
                token_map: input.token_map,
                overlay_line_starts: input.overlay_line_starts,
                source_line_starts: input.source_line_starts,
                overlay_text,
                source_text,
                identity_map: matches!(input.kind, InputKind::KitFile | InputKind::UserTsOverlay),
                ignore_regions,
            },
        );
        // Only in-scope Svelte files + Kit overlays land in the
        // tsconfig's `files` list. SvelteAuxiliary overlays are
        // reachable via import resolution from the listed entries
        // (through the ambient sidecar + rootDirs merge) — listing
        // them in `files` would pull them into the program
        // unconditionally, re-introducing the out-of-scope-error
        // noise that the tsconfig scope filter exists to prevent.
        if !matches!(input.kind, InputKind::SvelteAuxiliary) {
            generated_paths.push(gen_path);
        }
    }

    // Step 1a: garbage-collect orphaned overlay files. A `.svelte`
    // source that was deleted or renamed leaves its stale overlay
    // (`Foo.svelte.svn.ts`) and ambient sidecar (`Foo.d.svelte.ts`)
    // behind in the cache; subsequent runs see them resolve as if
    // the source still existed, masking the user's real "module not
    // found" error at the import site.
    //
    // Walk the cache's `svelte/` subtree and delete any file that
    // wasn't written this run. `written_paths` accumulated every
    // gen_path + ambient_path the loop above produced; anything else
    // under `svelte_dir` is orphaned. Best-effort delete — a held
    // file (Windows process lock, antivirus scan) leaves the orphan
    // for next run and doesn't break this one.
    gc_orphaned_overlays(&layout.svelte_dir, &written_paths);

    // Step 1b: write the synthetic `.svelte-kit/types/` mirror so the
    // `$types.d.ts` chain `'../(…/)src/routes/…/+page.js'` resolves
    // through our typed Kit-file copies instead of the user's untyped
    // source. Returns Some(mirror_dir) when the user actually has a
    // svelte-kit-generated types tree (the common SvelteKit case),
    // None for non-Kit projects. The overlay builder uses this signal
    // to enable rootDirs priority + include-glob redirect; without it
    // the overlay degrades cleanly to today's behavior.
    let kit_types_mirror = kit_types_mirror::sync_mirror(&layout)?;

    // Step 1c: write fallback `$app/*` ambient-module declarations
    // when this is a Kit project (`.svelte-kit/types/` exists) but
    // `@sveltejs/kit` types aren't reachable from the workspace's
    // node_modules. Closes TS2307 on `import { dev } from
    // '$app/environment'` for monorepos that have Kit at the root
    // but not in per-app node_modules. Returns None when the
    // fallback isn't needed (real types win, or non-Kit project).
    let kit_app_ambients = kit_app_ambients::write_ambients(&layout)?;

    // Step 2: write overlay tsconfig.
    let overlay = overlay::build(
        &layout,
        user_tsconfig,
        &generated_paths,
        &kit_overlay_sources,
        kit_types_mirror.as_deref(),
        kit_app_ambients.as_deref(),
    );
    let overlay_text = serde_json::to_string_pretty(&overlay)?;
    write_if_changed(&layout.overlay_tsconfig, &overlay_text)?;

    // Step 3: spawn tsgo.
    let tsgo = discover(workspace)?;
    let run = run_tsgo(
        &tsgo,
        &layout.overlay_tsconfig,
        workspace,
        extended_diagnostics,
    )?;

    // Step 4: map diagnostics back to source paths + apply line map.
    // Drop diagnostics that are about our overlay tsconfig itself —
    // those are noise from compiler options we set deliberately
    // (e.g. TS5102 baseUrl deprecation; we filter rather than re-add
    // baseUrl because doing so suppresses tsgo's diagnostic emission
    // on our overlay files entirely).
    let diagnostics = run
        .diagnostics
        .into_iter()
        .filter(|d| !filters::is_overlay_tsconfig_noise(d, &layout))
        .filter_map(|d| map_diagnostic(d, &layout, &map_data))
        .filter(|d| !filters::is_svelte4_reactive_noop_comma(d))
        .collect();
    Ok(CheckOutput {
        diagnostics,
        extended_diagnostics: run.extended_diagnostics,
    })
}

fn map_diagnostic(
    raw: RawDiagnostic,
    layout: &CacheLayout,
    map_data: &std::collections::HashMap<PathBuf, MapData>,
) -> Option<CheckDiagnostic> {
    // tsgo emits paths relative to the working directory when the input
    // tsconfig path is itself relative (which it usually is). Absolutize
    // against the workspace root so cache-layout lookups work uniformly.
    //
    // tsgo's relative paths can include `..` segments — e.g. when the
    // overlay tsconfig lives below the workspace, tsgo reports
    // `../../node_modules/.cache/svelte-check-native/svelte/Foo.svelte.svn.ts`.
    // After `workspace.join(raw.file)`, the resulting path is
    // syntactically `/ws/../../node_modules/.cache/.../Foo.svelte.svn.ts`
    // — the `map_data` HashMap key (registered as the canonical cache
    // path) won't lex-match the unnormalised join. Lexically resolve
    // `..` and `.` components so both sides converge.
    let absolute_file = if raw.file.is_absolute() {
        raw.file.clone()
    } else {
        layout.workspace.join(&raw.file)
    };
    let absolute_file = path_utils::lexical_normalise(&absolute_file);
    let (source_path, line, column) = match layout.original_from_generated(&absolute_file) {
        Some(orig) => {
            // For overlay files, require the position to resolve to a
            // verbatim user-source origin OR a token-map entry.
            // Diagnostics against synthesized scaffolding with no map
            // entry (component ctor calls, default-export type,
            // wrapper, void blocks) are dropped — mirrors upstream
            // svelte-check's source-map-driven filter. Without this,
            // bench repos using libraries with complex Prop unions
            // (bits-ui, shadcn-style) surface dozens of false
            // positives against synthesized `new $$_C({...})` sites
            // that upstream silently filters.
            let data = map_data.get(&absolute_file)?;
            // Ignore-region filter: if the diagnostic's overlay byte
            // position falls inside a `/*svn:ignore_start*/…
            // /*svn:ignore_end*/` region, drop it. Mirrors upstream
            // `isInGeneratedCode` at
            // `language-server/src/plugins/typescript/features/
            // utils.ts:102-109`. Wrapping emit-synthesised scaffolding
            // in these markers at emit time lets this filter drop
            // false-positive diagnostics that would otherwise surface
            // on overlay bytes the user never wrote.
            if let Some(offset) = position::overlay_byte_offset(data, raw.line, raw.column)
                && filters::is_in_ignore_region(&data.ignore_regions, offset)
            {
                return None;
            }
            // SVELTE-4-COMPAT: drop TS7028 ("Unused label") on the `$`
            // identifier that prefixes a Svelte-4 reactive `$:`
            // statement. Both we and upstream wrap unhandled reactive
            // expressions in `;() => { $: <expr> }` so tsgo type-checks
            // the body; the inner `$:` label is structural — not a
            // real label the user wrote — but tsgo flags it under
            // `allowUnusedLabels: false` (set in many real tsconfigs,
            // including all of threlte's packages). Mirrors upstream
            // svelte-check's `isUnusedReactiveStatementLabel` filter at
            // `language-tools/packages/language-server/src/plugins/
            // typescript/features/DiagnosticsProvider.ts:476-495`.
            if raw.code == 7028
                && let Some(offset) = position::overlay_byte_offset(data, raw.line, raw.column)
                && filters::is_overlay_dollar_reactive_label(&data.overlay_text, offset)
            {
                return None;
            }
            // Drop TS1117 ("multiple properties same name") and
            // TS2300 ("Duplicate identifier") when the overlay byte
            // position is on an Element attribute name. Upstream's
            // equivalent filter at
            // `language-server/src/plugins/typescript/features/
            // DiagnosticsProvider.ts:360-374` checks the Svelte AST
            // node — `isAttributeName(node, 'Element') ||
            // isEventHandler(node, 'Element')`. We don't carry the
            // Svelte AST through to the diagnostic mapper, so the
            // check here is a structural overlay scan: the user
            // idiom `<el on:click={fn} on:click>` (handle + forward)
            // produces duplicate `"on:NAME"` keys, and the spread-
            // plus-attribute idiom `<el {...spread} class={x}>` can
            // produce other duplicate keys when the spread also
            // contains `class`. Both manifest in the overlay as a
            // quoted-string property name in a `createElement` arg
            // literal. The `"` prefix narrows the false-positive
            // surface — script-side identifier collisions keep
            // firing TS2300 because their identifiers aren't quoted.
            //
            // Less general than upstream's AST check (we don't catch
            // unquoted shorthand keys in element literals), but
            // covers every real-world pattern observed on benches
            // through 2026-04-27.
            if (raw.code == 1117 || raw.code == 2300)
                && let Some(offset) = position::overlay_byte_offset(data, raw.line, raw.column)
                && filters::is_overlay_attribute_key(&data.overlay_text, offset)
            {
                return None;
            }
            match position::translate_position(data, raw.line, raw.column) {
                Some((mapped_line, mapped_col)) => (orig, mapped_line, mapped_col),
                None => return None,
            }
        }
        None => (absolute_file, raw.line, raw.column),
    };
    let span = raw.span_length.unwrap_or(0);
    Some(CheckDiagnostic {
        source_path,
        line,
        column,
        // tsgo emits a single-line span_length, no end-line info — so
        // for TS diagnostics we collapse end_line == start_line.
        end_line: line,
        end_column: column.saturating_add(span),
        severity: raw.severity,
        code: DiagnosticCode::Numeric(raw.code),
        message: raw.message,
        // Both TS and JS diagnostics from tsgo are classified as `js`
        // by upstream svelte-check (same backend).
        source: DiagnosticSource::Js,
        // tsgo doesn't supply doc URLs in its compact output; we'd
        // need a static lookup table per error code to fill these in
        // (typescript.tv has them but mapping isn't 1-to-1). Leave
        // empty for now — IDEs that want links can derive them from
        // the numeric code.
        code_description_url: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    // Tests of moved helpers reference them via their new module
    // paths. Pulled in here so the test bodies stay verbatim.
    use crate::filters::{is_in_ignore_region, scan_ignore_regions};
    use crate::path_utils::lexical_normalise;
    use crate::position::{
        byte_to_position, find_tightest_token, overlay_byte_offset, position_to_byte,
        translate_position,
    };

    #[test]
    fn lexical_normalise_resolves_dot_dot_segments() {
        // Common case: workspace.join(rel_with_dotdot) lands a path that
        // syntactically contains `..` and `.` segments — normalise pops
        // them off without touching the filesystem.
        assert_eq!(
            lexical_normalise(Path::new("/ws/foo/../bar/Baz.ts")),
            PathBuf::from("/ws/bar/Baz.ts")
        );
        assert_eq!(
            lexical_normalise(Path::new("/ws/./foo/Bar.ts")),
            PathBuf::from("/ws/foo/Bar.ts")
        );
        // Pop chain shouldn't go past the root.
        assert_eq!(
            lexical_normalise(Path::new("/foo/../../bar/Baz.ts")),
            PathBuf::from("/bar/Baz.ts")
        );
        // Leading `..` chain on a relative path is preserved (no
        // anchor to pop against).
        assert_eq!(
            lexical_normalise(Path::new("../../foo/Bar.ts")),
            PathBuf::from("../../foo/Bar.ts")
        );
        // No-op on already-clean paths.
        assert_eq!(
            lexical_normalise(Path::new("/ws/foo/Bar.ts")),
            PathBuf::from("/ws/foo/Bar.ts")
        );
    }

    #[test]
    fn shim_keeps_fallback_when_asked() {
        let full = resolve_shim_text(true);
        assert!(full.contains(FALLBACK_BEGIN));
        assert!(full.contains(FALLBACK_END));
        assert!(full.contains("declare module 'svelte'"));
        assert!(full.contains("declare module 'svelte/elements'"));
        // Core runes must remain.
        assert!(full.contains("$state"));
    }

    #[test]
    fn shim_strips_fallback_when_real_svelte_present() {
        let stripped = resolve_shim_text(false);
        assert!(!stripped.contains("declare module 'svelte'"));
        assert!(!stripped.contains("declare module 'svelte/elements'"));
        // Core runes still present.
        assert!(stripped.contains("$state"));
        // Line count preserved so diagnostic positions in the shim
        // stay stable across the two modes.
        let full_lines = resolve_shim_text(true).lines().count();
        let stripped_lines = stripped.lines().count();
        assert_eq!(full_lines, stripped_lines);
    }

    #[test]
    fn scan_ignore_regions_paired() {
        let text = "line1\n/*svn:ignore_start*/inside/*svn:ignore_end*/outside\n".to_string();
        let regions = scan_ignore_regions(&text);
        // The scanned region covers bytes from END of start-marker to
        // START of end-marker — i.e. just "inside".
        assert_eq!(regions.len(), 1);
        let (start, end) = regions[0];
        let inside = &text[start as usize..end as usize];
        assert_eq!(inside, "inside");
    }

    #[test]
    fn scan_ignore_regions_unmatched_start_extends_to_eof() {
        let text = "/*svn:ignore_start*/dangling".to_string();
        let regions = scan_ignore_regions(&text);
        assert_eq!(regions.len(), 1);
        let (start, end) = regions[0];
        assert_eq!(end as usize, text.len());
        assert_eq!(&text[start as usize..end as usize], "dangling");
    }

    #[test]
    fn scan_ignore_regions_multiple_non_overlapping() {
        let text =
            "a /*svn:ignore_start*/X/*svn:ignore_end*/ b /*svn:ignore_start*/Y/*svn:ignore_end*/ c"
                .to_string();
        let regions = scan_ignore_regions(&text);
        assert_eq!(regions.len(), 2);
        assert_eq!(&text[regions[0].0 as usize..regions[0].1 as usize], "X");
        assert_eq!(&text[regions[1].0 as usize..regions[1].1 as usize], "Y");
    }

    #[test]
    fn scan_ignore_regions_no_markers_returns_empty() {
        let text = "plain overlay with no markers\n".to_string();
        assert!(scan_ignore_regions(&text).is_empty());
    }

    #[test]
    fn is_in_ignore_region_boundary_semantics() {
        let regions = vec![(10u32, 20u32)];
        // Exclusive end: 20 is NOT inside.
        assert!(is_in_ignore_region(&regions, 10));
        assert!(is_in_ignore_region(&regions, 15));
        assert!(is_in_ignore_region(&regions, 19));
        assert!(!is_in_ignore_region(&regions, 20));
        assert!(!is_in_ignore_region(&regions, 9));
    }

    #[test]
    fn overlay_byte_offset_one_based_lines_and_columns() {
        let data = MapData {
            // line 1 = "line1" (5 bytes + newline = 6), line 2 = "line2".
            overlay_line_starts: vec![0, 6, 11],
            overlay_text: "line1\nline2".to_string(),
            ..Default::default()
        };
        // (1, 1) == byte 0 (start of line 1).
        assert_eq!(overlay_byte_offset(&data, 1, 1), Some(0));
        // (1, 4) == byte 3.
        assert_eq!(overlay_byte_offset(&data, 1, 4), Some(3));
        // (2, 1) == byte 6.
        assert_eq!(overlay_byte_offset(&data, 2, 1), Some(6));
    }

    fn line_maps_for(path: &str, entries: Vec<LineMapEntry>) -> HashMap<PathBuf, MapData> {
        let mut m = HashMap::new();
        m.insert(
            PathBuf::from(path),
            MapData {
                line_map: entries,
                ..Default::default()
            },
        );
        m
    }

    /// Test helper: build a MapData HashMap with a token map and the
    /// overlay/source line-starts indices sized to cover the byte
    /// offsets referenced in the test.
    fn token_maps_for(
        path: &str,
        token_entries: Vec<TokenMapEntry>,
        overlay_line_starts: Vec<u32>,
        source_line_starts: Vec<u32>,
    ) -> HashMap<PathBuf, MapData> {
        let mut m = HashMap::new();
        m.insert(
            PathBuf::from(path),
            MapData {
                token_map: token_entries,
                overlay_line_starts,
                source_line_starts,
                ..Default::default()
            },
        );
        m
    }

    #[test]
    fn maps_generated_file_back_to_source_via_line_map() {
        let layout = CacheLayout::for_workspace("/proj");
        let gen_path = "/proj/.svelte-check/svelte/src/Foo.svelte.svn.ts";
        // overlay lines 5..15 map to source lines 1..11.
        let map = line_maps_for(
            gen_path,
            vec![LineMapEntry {
                overlay_start_line: 5,
                overlay_end_line: 15,
                source_start_line: 1,
            }],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 10,
            column: 5,
            severity: Severity::Error,
            code: 2322,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &map).expect("mapped");
        assert_eq!(mapped.source_path, PathBuf::from("/proj/src/Foo.svelte"));
        // overlay line 10 - overlay_start (5) = 5, + source_start (1) = 6.
        assert_eq!(mapped.line, 6);
    }

    #[test]
    fn passes_through_non_generated_files() {
        let layout = CacheLayout::for_workspace("/proj");
        let raw = RawDiagnostic {
            file: PathBuf::from("/proj/src/plain.ts"),
            line: 4,
            column: 1,
            severity: Severity::Error,
            code: 1000,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &HashMap::new()).expect("mapped");
        assert_eq!(mapped.source_path, PathBuf::from("/proj/src/plain.ts"));
        assert_eq!(mapped.line, 4); // no offset applied to non-generated files
    }

    #[test]
    fn diagnostics_outside_any_mapped_range_are_dropped() {
        // Synthesized lines (header, function wrapper, void block,
        // component ctor scaffolding) have no user-source origin.
        // Dropping mirrors upstream svelte-check's source-map filter.
        let gen_path = "/proj/.svelte-check/svelte/++X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![LineMapEntry {
                overlay_start_line: 10,
                overlay_end_line: 20,
                source_start_line: 5,
            }],
        );
        let raw_before = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 1,
            column: 1,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        assert!(map_diagnostic(raw_before, &layout, &map).is_none());
        let raw_after = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 30,
            column: 1,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        assert!(map_diagnostic(raw_after, &layout, &map).is_none());
    }

    #[test]
    fn maps_between_multiple_mapped_ranges_uses_matching_range() {
        // Three contiguous source regions map to three overlay regions.
        // A diagnostic inside the second overlay range maps through the
        // second range's source-start, not the first or third.
        let gen_path = "/proj/.svelte-check/svelte/++X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![
                LineMapEntry {
                    overlay_start_line: 1,
                    overlay_end_line: 3,
                    source_start_line: 1,
                },
                LineMapEntry {
                    overlay_start_line: 10,
                    overlay_end_line: 20,
                    source_start_line: 50,
                },
                LineMapEntry {
                    overlay_start_line: 30,
                    overlay_end_line: 40,
                    source_start_line: 100,
                },
            ],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 15,
            column: 0,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        // 15 is in range [10, 20) — offset 5 from overlay_start (10),
        // applied to source_start (50) = source line 55.
        assert_eq!(map_diagnostic(raw, &layout, &map).expect("mapped").line, 55);
    }

    #[test]
    fn maps_between_gaps_drops_the_diagnostic() {
        // A diagnostic in the gap between mapped ranges has no
        // user-source origin — drop it (was previously clamped).
        let gen_path = "/proj/.svelte-check/svelte/++X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![
                LineMapEntry {
                    overlay_start_line: 1,
                    overlay_end_line: 3,
                    source_start_line: 1,
                },
                LineMapEntry {
                    overlay_start_line: 10,
                    overlay_end_line: 20,
                    source_start_line: 50,
                },
            ],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 5, // in the gap between [1,3) and [10,20)
            column: 0,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        assert!(map_diagnostic(raw, &layout, &map).is_none());
    }

    #[test]
    fn empty_line_map_passes_line_through() {
        // When a generated file has no line-map entries at all, the
        // mapper falls through to "line stays as tsgo reported it"
        // rather than erroring.
        let gen_path = "/proj/.svelte-check/svelte/src/X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(gen_path, Vec::new());
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 42,
            column: 0,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &map);
        // Empty line map for an overlay file now drops the diagnostic
        // entirely (same principle as outside-any-range: no evidence
        // the tsgo diagnostic originated from user source).
        assert!(mapped.is_none());
    }

    #[test]
    fn generated_path_reverse_maps_to_source_even_without_line_map() {
        // The path-reverse mapping is independent of the line map —
        // a file we emitted lives at <cache>/svelte/<rel>/<name>.svelte.ts
        // and reverses to <workspace>/<rel>/<name>.svelte regardless of
        // whether tsgo's diagnostic has a corresponding line-map entry.
        let gen_path = "/proj/.svelte-check/svelte/lib/components/Foo.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 1,
            column: 1,
            severity: Severity::Error,
            code: 1,
            message: "x".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &HashMap::new());
        // Without any line-map entry, diagnostics against an overlay
        // file are dropped — the path-reverse logic itself still
        // works but there's no user-source line to attribute to.
        assert!(mapped.is_none());
    }

    #[test]
    fn column_and_severity_and_code_pass_through_unchanged() {
        // The mapper only rewrites path and line. Column, severity,
        // code, and message must pass through verbatim.
        let gen_path = "/proj/.svelte-check/svelte/src/X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        let map = line_maps_for(
            gen_path,
            vec![LineMapEntry {
                overlay_start_line: 5,
                overlay_end_line: 15,
                source_start_line: 1,
            }],
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 7,
            column: 42,
            severity: Severity::Warning,
            code: 2345,
            message: "original message".to_string(),
            span_length: None,
        };
        let mapped = map_diagnostic(raw, &layout, &map).expect("mapped");
        assert_eq!(mapped.column, 42);
        assert_eq!(mapped.severity, Severity::Warning);
        assert!(
            matches!(mapped.code, DiagnosticCode::Numeric(2345)),
            "numeric code should survive the mapper unchanged; got {:?}",
            mapped.code
        );
        assert_eq!(mapped.message, "original message");
    }

    #[test]
    fn cache_layout_generated_path_round_trips_through_reverse() {
        // Round-trip: a .svelte path → generated_path → original_from_generated
        // must return the same .svelte path. This is the invariant the
        // diagnostic mapper depends on, exercised here independent of
        // map_diagnostic itself.
        let layout = CacheLayout::for_workspace("/proj");
        let svelte_path = PathBuf::from("/proj/src/lib/components/Button.svelte");
        let generated = layout.generated_path(&svelte_path);
        let back = layout
            .original_from_generated(&generated)
            .expect("reverse mapping must succeed for a path we generated");
        assert_eq!(back, svelte_path);
    }

    #[test]
    fn cache_layout_reverse_rejects_paths_outside_svelte_dir() {
        // Paths that don't live under <cache>/svelte/ aren't ours and
        // should reverse to None — the mapper then skips the rewrite
        // and passes the path through as-is.
        let layout = CacheLayout::for_workspace("/proj");
        assert!(
            layout
                .original_from_generated(Path::new("/proj/src/plain.ts"))
                .is_none()
        );
        assert!(
            layout
                .original_from_generated(Path::new("/elsewhere/X.ts"))
                .is_none()
        );
    }

    // --- TokenMapEntry / translate_position coverage ------------------
    //
    // Sanity-floor tests for the byte-span machinery introduced in v0.3
    // Item 1. Verbatim-block LineMapEntry behavior is tested above;
    // these exercise the new token-map path, the fallback chain
    // (token miss → line-map → drop), the "tightest wins" rule, and
    // the helper conversion functions (position_to_byte /
    // byte_to_position) in isolation.

    #[test]
    fn position_to_byte_handles_line_and_column_correctly() {
        // Overlay buffer: "ab\ncd\nef" (lines 1-3). Line starts:
        //   line 1 @ 0, line 2 @ 3, line 3 @ 6, sentinel @ 8.
        let text = "ab\ncd\nef";
        let starts = svn_emit::compute_line_starts(text);
        assert_eq!(starts, vec![0, 3, 6, 8]);
        // (1,1) → byte 0 (line 1 col 1 = first char).
        assert_eq!(position_to_byte(&starts, text, 1, 1), Some(0));
        // (2,1) → byte 3 (first char of line 2).
        assert_eq!(position_to_byte(&starts, text, 2, 1), Some(3));
        // (2,2) → byte 4.
        assert_eq!(position_to_byte(&starts, text, 2, 2), Some(4));
        // Column past end of line clamps within the buffer.
        assert!(position_to_byte(&starts, text, 2, 99).unwrap() <= 8);
        // Line past EOF returns None.
        assert_eq!(position_to_byte(&starts, text, 99, 1), None);
        // Line 0 is invalid (we're 1-based).
        assert_eq!(position_to_byte(&starts, text, 0, 1), None);
    }

    #[test]
    fn position_to_byte_handles_utf16_columns() {
        // `é` is 2 UTF-8 bytes but 1 UTF-16 unit. The 1-based UTF-16
        // column N points at the N-th *char* in the line; conversion
        // returns its starting byte offset.
        // text bytes: c(1) a(1) f(1) é(2) ' '(1) x(1) x(1) = 7 bytes
        let text = "café xx";
        let starts = svn_emit::compute_line_starts(text);
        assert_eq!(position_to_byte(&starts, text, 1, 1), Some(0)); // 'c' — byte 0
        assert_eq!(position_to_byte(&starts, text, 1, 4), Some(3)); // 'é' — byte 3 (after caf)
        assert_eq!(position_to_byte(&starts, text, 1, 5), Some(5)); // ' ' — byte 5 (after é)
        assert_eq!(position_to_byte(&starts, text, 1, 6), Some(6)); // 'x'
        assert_eq!(position_to_byte(&starts, text, 1, 7), Some(7)); // last 'x'

        // Astral char (4-byte UTF-8, 2 UTF-16 units) — `🎉` U+1F389.
        // bytes: 🎉(4) e(1) n(1) d(1) = 7 bytes
        // UTF-16 cols: cols 1-2 cover 🎉, col 3 = 'e', etc.
        let astral = "🎉end";
        let starts = svn_emit::compute_line_starts(astral);
        assert_eq!(position_to_byte(&starts, astral, 1, 1), Some(0)); // 🎉 start
        assert_eq!(position_to_byte(&starts, astral, 1, 3), Some(4)); // 'e' — after 2 UTF-16 units = 4 bytes in
    }

    #[test]
    fn byte_to_position_is_inverse_of_position_to_byte_within_lines() {
        // Round-trip: (line, col) → byte → (line, col). Every position
        // inside a line must round-trip exactly. Tests the buffer
        // "abc\ndef\nghi" (3 lines of 3 chars each).
        let text = "abc\ndef\nghi";
        let starts = svn_emit::compute_line_starts(text);
        for line in 1..=3 {
            for col in 1..=3 {
                let byte = position_to_byte(&starts, text, line, col).unwrap();
                let (l, c) = byte_to_position(&starts, text, byte);
                assert_eq!(
                    (l, c),
                    (line, col),
                    "round-trip failed at line {} col {}: byte {} → ({}, {})",
                    line,
                    col,
                    byte,
                    l,
                    c
                );
            }
        }
    }

    #[test]
    fn byte_to_position_emits_utf16_columns() {
        // Inverse of the position_to_byte UTF-16 test: a byte offset
        // inside / after a multi-byte char must produce a UTF-16
        // column count, not a byte column.
        // text bytes: c(0) a(1) f(2) é=2 bytes (3,4) ' '(5) x(6) x(7)
        let text = "café xx";
        let starts = svn_emit::compute_line_starts(text);
        // Byte 0 = 'c' → col 1 (1-based).
        assert_eq!(byte_to_position(&starts, text, 0), (1, 1));
        // Byte 5 = ' ' → 4 chars consumed (c, a, f, é) → col 5.
        assert_eq!(byte_to_position(&starts, text, 5), (1, 5));
        // Byte 7 = second 'x' (last char start) → 6 chars consumed → col 7.
        assert_eq!(byte_to_position(&starts, text, 7), (1, 7));
    }

    #[test]
    fn find_tightest_token_prefers_smallest_span() {
        // Three nested spans at the same byte: outer [0, 100), middle
        // [10, 50), inner [20, 30). Byte 25 is inside all three.
        // tightest = [20, 30) (width 10).
        let map = vec![
            TokenMapEntry {
                overlay_byte_start: 0,
                overlay_byte_end: 100,
                source_byte_start: 0,
                source_byte_end: 100,
            },
            TokenMapEntry {
                overlay_byte_start: 10,
                overlay_byte_end: 50,
                source_byte_start: 200,
                source_byte_end: 240,
            },
            TokenMapEntry {
                overlay_byte_start: 20,
                overlay_byte_end: 30,
                source_byte_start: 500,
                source_byte_end: 510,
            },
        ];
        let hit = find_tightest_token(&map, 25).expect("hit");
        assert_eq!(hit.overlay_byte_start, 20);
        assert_eq!(hit.source_byte_start, 500);
    }

    #[test]
    fn find_tightest_token_returns_none_when_no_span_contains() {
        let map = vec![TokenMapEntry {
            overlay_byte_start: 10,
            overlay_byte_end: 20,
            source_byte_start: 100,
            source_byte_end: 110,
        }];
        assert!(find_tightest_token(&map, 5).is_none());
        assert!(find_tightest_token(&map, 20).is_none()); // end is exclusive
        assert!(find_tightest_token(&map, 100).is_none());
    }

    #[test]
    fn translate_position_maps_line_and_column_via_token_map() {
        // Overlay buffer "aa\nBBBBBB\ncc" — line 2 is synthesized
        // scaffolding splicing in source bytes [42, 48).
        // Overlay line 2 col 3 = byte 5. That's offset 2 inside the
        // token span [3, 9), so source byte = 42 + 2 = 44.
        // Source buffer "LINE1\nLINE2\nLINE3" — byte 44 would land
        // somewhere mid-line for a larger source; here we use
        // synthetic line-starts for clarity.
        let data = MapData {
            token_map: vec![TokenMapEntry {
                overlay_byte_start: 3,
                overlay_byte_end: 9,
                source_byte_start: 42,
                source_byte_end: 48,
            }],
            // Overlay: "aa\nBBBBBB\ncc" → starts [0, 3, 10, 12].
            overlay_line_starts: vec![0, 3, 10, 12],
            overlay_text: "aa\nBBBBBB\ncc".to_string(),
            // Source: line 5 starts at byte 40. Byte 44 is line 5 col
            // 5 (0-offset 4 from line start → 1-based col 5). Provide
            // a 60-byte filler text so byte_to_position can count
            // chars between line start and target byte.
            source_line_starts: vec![0, 10, 20, 30, 40, 50, 60],
            source_text: "0123456789".repeat(6),
            ..Default::default()
        };
        // Overlay (line=2, col=3) corresponds to overlay byte 3+2=5.
        let (line, col) = translate_position(&data, 2, 3).expect("mapped");
        // Source byte 44 → line 5 (starts at 40), col 5.
        assert_eq!((line, col), (5, 5));
    }

    #[test]
    fn translate_position_falls_back_to_line_map_on_token_miss() {
        // Token map is empty but line map covers overlay lines 5..15
        // mapping to source lines 1..11. Column must pass through
        // unchanged — verbatim script content emits at the same
        // column in overlay and source.
        let data = MapData {
            line_map: vec![LineMapEntry {
                overlay_start_line: 5,
                overlay_end_line: 15,
                source_start_line: 1,
            }],
            ..Default::default()
        };
        let (line, col) = translate_position(&data, 10, 42).expect("mapped");
        assert_eq!(line, 6); // 10 - 5 + 1
        assert_eq!(col, 42); // unchanged
    }

    #[test]
    fn translate_position_drops_when_neither_map_covers() {
        // A diagnostic that doesn't match a token entry and doesn't
        // fall inside any LineMapEntry range is dropped — matches
        // upstream svelte-check's source-map-driven filter.
        let data = MapData {
            line_map: vec![LineMapEntry {
                overlay_start_line: 5,
                overlay_end_line: 15,
                source_start_line: 1,
            }],
            token_map: vec![TokenMapEntry {
                overlay_byte_start: 0,
                overlay_byte_end: 10,
                source_byte_start: 0,
                source_byte_end: 10,
            }],
            overlay_line_starts: vec![0, 20, 40, 60],
            source_line_starts: vec![0, 20, 40, 60],
            ..Default::default()
        };
        // Line 3 (overlay byte ~40) — outside the token span [0, 10)
        // AND outside the line-map range [5, 15).
        assert!(translate_position(&data, 3, 1).is_none());
    }

    #[test]
    fn map_diagnostic_rewrites_both_line_and_column_via_token_map() {
        // End-to-end through map_diagnostic: given a token hit, the
        // returned CheckDiagnostic must have the mapped line AND the
        // mapped column — not the original tsgo column. Regression
        // guard for the column rewrite introduced in this item.
        let gen_path = "/proj/.svelte-check/svelte/src/X.svelte.ts";
        let layout = CacheLayout::for_workspace("/proj");
        // Overlay line 2 col 3 = byte 5 (see test above). Token span
        // maps to source bytes [42, 48), inside source line 5.
        let mut m = HashMap::new();
        m.insert(
            PathBuf::from(gen_path),
            MapData {
                token_map: vec![TokenMapEntry {
                    overlay_byte_start: 3,
                    overlay_byte_end: 9,
                    source_byte_start: 42,
                    source_byte_end: 48,
                }],
                overlay_line_starts: vec![0, 3, 10, 12],
                overlay_text: "aa\nBBBBBB\ncc".to_string(),
                source_line_starts: vec![0, 10, 20, 30, 40, 50, 60],
                source_text: "0123456789".repeat(6),
                ..Default::default()
            },
        );
        let raw = RawDiagnostic {
            file: PathBuf::from(gen_path),
            line: 2,
            column: 3,
            severity: Severity::Error,
            code: 2345,
            message: "mismatch".to_string(),
            span_length: Some(4),
        };
        let mapped = map_diagnostic(raw, &layout, &m).expect("mapped");
        assert_eq!(mapped.line, 5, "line must follow the token span");
        assert_eq!(mapped.column, 5, "column must follow the token span");
        assert_eq!(mapped.end_column, 9, "end column = column + span_length");
    }

    #[test]
    fn token_maps_for_helper_is_consumed() {
        // Smoke test for the token_maps_for builder the other token
        // tests above rely on indirectly; keeps the helper honest.
        let map = token_maps_for(
            "/proj/.svelte-check/svelte/src/Y.svelte.ts",
            vec![TokenMapEntry {
                overlay_byte_start: 0,
                overlay_byte_end: 1,
                source_byte_start: 0,
                source_byte_end: 1,
            }],
            vec![0, 1],
            vec![0, 1],
        );
        assert_eq!(map.len(), 1);
        let data = map.values().next().unwrap();
        assert_eq!(data.token_map.len(), 1);
        assert_eq!(data.overlay_line_starts.len(), 2);
    }
}
