//! Narrow-scope Svelte language server — spike.
//!
//! Proves the design measured in the architecture note: instead of holding a
//! TypeScript program for the whole workspace, build one for the open file's
//! import closure only. Diagnostics are produced by the same emit and the same
//! diagnostic mapper the CLI uses, so positions are real, not approximated.
//!
//! Scope of this spike: diagnostics on open/save. No hover, no completion —
//! those need a warm `tsgo --lsp` child and a source->overlay reverse map,
//! which is stage 2.
//!
//! Run `svelte-lsp-spike --selftest <file.svelte>` to exercise the whole path
//! once and print timings, without an editor attached.

mod closure;
mod diagnostics;
mod position;
mod rpc;
mod scope;
mod scope_build;
mod tsgo;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--selftest") => match args.get(1) {
            Some(file) => selftest(Path::new(file)),
            None => {
                eprintln!("--selftest needs a .svelte file path");
                std::process::exit(2);
            }
        },
        _ => serve(),
    }
}

/// One full pass with no editor attached: closure -> emit -> tsgo -> mapped
/// diagnostics, with the timings that matter printed to stderr.
fn selftest(file: &Path) {
    let file = match file.canonicalize() {
        Ok(p) => p,
        Err(err) => {
            eprintln!("cannot read {}: {err}", file.display());
            std::process::exit(2);
        }
    };
    let Some(project) = scope::Project::discover(&file) else {
        eprintln!("no tsconfig.json or jsconfig.json at or above {}", file.display());
        std::process::exit(2);
    };
    eprintln!("workspace     {}", project.workspace.display());
    eprintln!("user tsconfig {}", project.user_tsconfig.display());

    let mut warm = None;
    let mut last_scope = Vec::new();
    for round in 1..=2 {
        let t0 = std::time::Instant::now();
        let closure = closure::compute(&file, &project.workspace);
        let t_closure = t0.elapsed();

        let t1 = std::time::Instant::now();
        let result = diagnostics::check_scope(
            &project,
            &closure,
            &HashMap::new(),
            &mut warm,
            &mut last_scope,
            // The CLI drops hints without --include-suggestions, and this mode
            // exists to be diffed against the CLI.
            false,
        );
        let t_check = t1.elapsed();

        match result {
            Ok((diags, _scope)) => {
                let for_file = diags.iter().filter(|d| d.source_path == file).count();
                eprintln!(
                    "round {round}: closure {} files in {:?} · check {:?} · {} diagnostics ({} in the open file)",
                    closure.len(),
                    t_closure,
                    t_check,
                    diags.len(),
                    for_file
                );
                if round == 2 {
                    // Machine-comparable form for the open file, so a sweep can
                    // diff these against the CLI's own output without parsing
                    // prose.
                    let mut own: Vec<String> = diags
                        .iter()
                        .filter(|d| d.source_path == file)
                        .map(|d| format!("{}:{}", d.line, d.column))
                        .collect();
                    own.sort();
                    println!("DIAGS {}", own.join(","));
                    if std::env::var("SPIKE_ALL").is_ok() {
                        for d in diags.iter().filter(|d| d.source_path != file) {
                            eprintln!(
                                "  OTHER {}:{}:{} {}",
                                d.source_path.strip_prefix(&project.workspace).unwrap_or(&d.source_path).display(),
                                d.line, d.column,
                                d.message.lines().next().unwrap_or("")
                            );
                        }
                    }
                    for d in diags.iter().filter(|d| d.source_path == file).take(10) {
                        eprintln!("  {}:{} {}", d.line, d.column, d.message.lines().next().unwrap_or(""));
                    }
                }
            }
            Err(err) => eprintln!("round {round}: check failed: {err}"),
        }
    }
    eprintln!("peak rss {} MB", peak_rss_mb());
}

/// Peak RSS of this process, in MB. Read from `ps` — good enough for a spike,
/// and it keeps the dependency list at one crate.
fn peak_rss_mb() -> u64 {
    let out = std::process::Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output();
    out.ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .map(|kb| kb / 1024)
        .unwrap_or(0)
}

/// The LSP loop. Synchronous and single-threaded: a check takes a few hundred
/// milliseconds and nothing else is in flight, so a work queue would be
/// ceremony without benefit at this stage.
fn serve() {
    rpc::begin_serving();
    let mut open_docs: HashMap<PathBuf, String> = HashMap::new();
    // Open tabs, most recently touched first. The scope is the union of their
    // closures, and the cap drops from the tail.
    let mut open_order: Vec<PathBuf> = Vec::new();
    let mut projects: HashMap<PathBuf, scope::Project> = HashMap::new();
    let mut published: Vec<PathBuf> = Vec::new();
    // Emit maps from the last check of each file, and the warm tsgo child.
    let mut maps: HashMap<PathBuf, position::FileMaps> = HashMap::new();
    let mut overlays: HashMap<PathBuf, PathBuf> = HashMap::new();
    let mut warm: Option<tsgo::Tsgo> = None;
    let mut last_scope: Vec<PathBuf> = Vec::new();

    while let Some(msg) = rpc::read_message() {
        let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
        let id = msg.get("id").cloned();

        match method {
            "initialize" => {
                rpc::respond(
                    id,
                    serde_json::json!({
                        "capabilities": {
                            // Full-text sync: the spike re-emits the whole file
                            // anyway, so incremental deltas would buy nothing.
                            "textDocumentSync": { "openClose": true, "change": 1, "save": true },
                            "hoverProvider": true
                        },
                        "serverInfo": { "name": "svelte-lsp-spike", "version": "0.1.0" }
                    }),
                );
            }
            "initialized" => {}
            "shutdown" => rpc::respond(id, serde_json::Value::Null),
            "exit" => return,

            "textDocument/didOpen" | "textDocument/didSave" | "textDocument/didChange" => {
                let Some(uri) = msg
                    .pointer("/params/textDocument/uri")
                    .and_then(|u| u.as_str())
                else {
                    continue;
                };
                let Some(path) = rpc::uri_to_path(uri) else { continue };

                if let Some(text) = document_text(&msg, method) {
                    open_docs.insert(path.clone(), text);
                }
                if method == "textDocument/didChange" {
                    // Typing does not rebuild the scope. Diagnostics refresh on
                    // save, which is the frozen-workspace bargain.
                    continue;
                }

                let project = projects
                    .entry(path.clone())
                    .or_insert_with(|| scope::Project::discover(&path).unwrap_or_default());
                if project.user_tsconfig.as_os_str().is_empty() {
                    rpc::log(&format!("no tsconfig above {}", path.display()));
                    continue;
                }

                // Touching a file makes it the most recent tab.
                open_order.retain(|p| p != &path);
                open_order.insert(0, path.clone());

                let t0 = std::time::Instant::now();
                let closure = closure::union(&open_order, &project.workspace);
                let elapsed_closure = t0.elapsed();

                let t1 = std::time::Instant::now();
                match diagnostics::check_scope(
                    project,
                    &closure,
                    &open_docs,
                    &mut warm,
                    &mut last_scope,
                    // Editors show hints; upstream's language server always
                    // requests them.
                    true,
                ) {
                    Ok((diags, scope)) => {
                        maps.extend(scope.maps);
                        overlays.extend(scope.overlays);
                        rpc::log(&format!(
                            "{}: {} tabs, {} files in scope, closure {:?}, check {:?}, {} diagnostics, rss {} MB",
                            path.file_name().unwrap_or_default().to_string_lossy(),
                            open_order.len(),
                            closure.len(),
                            elapsed_closure,
                            t1.elapsed(),
                            diags.len(),
                            peak_rss_mb()
                        ));
                        publish_all(&diags, &closure, &mut published);
                    }
                    Err(err) => rpc::log(&format!("check failed: {err}")),
                }
            }

            "textDocument/didClose" => {
                if let Some(path) = msg
                    .pointer("/params/textDocument/uri")
                    .and_then(|u| u.as_str())
                    .and_then(rpc::uri_to_path)
                {
                    open_docs.remove(&path);
                    // The scope shrinks on the next check rather than now —
                    // closing a tab is not a reason to make the editor wait.
                    open_order.retain(|p| p != &path);
                }
            }

            "textDocument/hover" => {
                let result = hover(&msg, &overlays, &maps, &mut warm);
                rpc::respond(id, result.unwrap_or(serde_json::Value::Null));
            }

            _ => {
                // Any other request still needs an answer, or the client blocks.
                if id.is_some() {
                    rpc::respond(id, serde_json::Value::Null);
                }
            }
        }
    }
}

/// Pull the document text out of a didOpen or didChange notification.
fn document_text(msg: &serde_json::Value, method: &str) -> Option<String> {
    match method {
        "textDocument/didOpen" => msg
            .pointer("/params/textDocument/text")
            .and_then(|t| t.as_str())
            .map(str::to_owned),
        "textDocument/didChange" => msg
            .pointer("/params/contentChanges/0/text")
            .and_then(|t| t.as_str())
            .map(str::to_owned),
        _ => None,
    }
}

/// Publish diagnostics for every file in the scope, and clear any file we
/// published for last time that is no longer in it — otherwise stale squiggles
/// outlive the scope that produced them.
fn publish_all(
    diags: &[svn_typecheck::CheckDiagnostic],
    closure: &[PathBuf],
    published: &mut Vec<PathBuf>,
) {
    let mut by_file: HashMap<&Path, Vec<&svn_typecheck::CheckDiagnostic>> = HashMap::new();
    for d in diags {
        by_file.entry(d.source_path.as_path()).or_default().push(d);
    }
    for path in closure {
        let entries = by_file.remove(path.as_path()).unwrap_or_default();
        rpc::publish_diagnostics(path, &entries);
    }
    for path in published.iter() {
        if !closure.contains(path) {
            rpc::publish_diagnostics(path, &[]);
        }
    }
    *published = closure.to_vec();
}

/// Answer a hover by asking the warm tsgo child about the equivalent position
/// in the overlay, then translating its range back to `.svelte` coordinates.
///
/// Requires the file to have been checked at least once — that is what wrote
/// the overlay and recorded the maps. In an editor that always holds, because
/// didOpen checks before anyone can hover.
fn hover(
    msg: &serde_json::Value,
    overlays: &HashMap<PathBuf, PathBuf>,
    maps: &HashMap<PathBuf, position::FileMaps>,
    warm: &mut Option<tsgo::Tsgo>,
) -> Option<serde_json::Value> {
    let path = msg
        .pointer("/params/textDocument/uri")
        .and_then(|u| u.as_str())
        .and_then(rpc::uri_to_path)?;
    let source_pos = position::Pos {
        line: msg.pointer("/params/position/line")?.as_u64()? as u32,
        character: msg.pointer("/params/position/character")?.as_u64()? as u32,
    };
    let file_maps = maps.get(&path)?;
    let overlay = overlays.get(&path)?;
    let overlay_pos = file_maps.to_overlay(source_pos)?;

    // The child is already warm and already holding this overlay — the check
    // that ran on didOpen started it and synced every file in the scope.
    let child = warm.as_mut()?;

    let asked = std::time::Instant::now();
    let result = match child.hover(overlay, overlay_pos.line, overlay_pos.character) {
        Ok(result) => result,
        Err(err) => {
            rpc::log(&format!("hover: {err}"));
            return None;
        }
    };
    rpc::log(&format!(
        "hover {}:{} -> overlay {}:{} in {:?}",
        source_pos.line, source_pos.character, overlay_pos.line, overlay_pos.character, asked.elapsed()
    ));

    // tsgo answers in overlay coordinates; the range has to come back or the
    // editor highlights the wrong span.
    let mut result = result?;

    // Upstream separates the signature from its documentation with a markdown
    // rule; tsgo runs them together. Measured against upstream's own
    // HoverProvider expectations this was the only difference across 10 cases —
    // every type and position already matched.
    if let Some(value) = result.pointer("/contents/value").and_then(|v| v.as_str())
        && let Some((code, docs)) = value.split_once("\n```\n")
        && !docs.is_empty()
        && !docs.starts_with("---")
    {
        let joined = format!("{code}\n```\n---\n{docs}");
        result["contents"]["value"] = serde_json::Value::String(joined);
    }
    if let Some(range) = result.get("range").cloned() {
        let start = position::Pos {
            line: range.pointer("/start/line")?.as_u64()? as u32,
            character: range.pointer("/start/character")?.as_u64()? as u32,
        };
        let end = position::Pos {
            line: range.pointer("/end/line")?.as_u64()? as u32,
            character: range.pointer("/end/character")?.as_u64()? as u32,
        };
        match (file_maps.to_source(start), file_maps.to_source(end)) {
            (Some(s), Some(e)) if s.line == e.line && e.character >= s.character => {
                result["range"] = serde_json::json!({
                    "start": { "line": s.line, "character": s.character },
                    "end": { "line": e.line, "character": e.character }
                });
            }
            // A range that maps to two different source lines is a synthesized
            // construct with no single source span. Dropping it leaves the
            // hover text intact and lets the editor use the word under the
            // cursor, which is better than highlighting something unrelated.
            _ => {
                result.as_object_mut()?.remove("range");
            }
        }
    }
    Some(result)
}
