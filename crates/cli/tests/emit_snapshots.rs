//! Stage-1 snapshot tests for our emit.
//!
//! For each sample, runs our binary with `--emit-ts`, captures stdout,
//! and compares it to a checked-in `expected.emit.ts` snapshot. This
//! is the emit-shape gate — it tests **translation fidelity** without
//! involving tsgo at all. Upstream's svelte2tsx uses the same pattern
//! (their `expectedv2.js`); we mirror it against our own emit.
//!
//! ### Layout
//!
//! Inputs come from three corpora, walked in order:
//!
//! 1. `language-tools/packages/svelte2tsx/test/svelte2tsx/samples/*.v5/`
//!    — upstream's 63 Svelte-5 svelte2tsx samples.
//! 2. `language-tools/packages/svelte2tsx/test/htmlx2jsx/samples/`
//!    — ~147 template-control-flow samples, minus Svelte-4-only ones
//!    listed in SKIP_HTMLX.
//! 3. `fixtures/bugs/<NN>-<slug>/` — our own grey-box bug fixtures.
//!
//! Expected outputs live in our tree at
//! `crates/cli/tests/emit_snapshots/<corpus>/<sample>/expected.emit.ts`,
//! mirroring the input layout. Inputs are read-only (in a submodule
//! for corpora 1 and 2); expected outputs are ours to update as emit
//! evolves.
//!
//! ### Update mode
//!
//! `UPDATE_SNAPSHOTS=1 cargo test --test emit_snapshots` rewrites every
//! snapshot file in place. Use this to bootstrap new samples or to
//! accept a deliberate emit change. Review the `git diff` before
//! committing to confirm the change is intentional.
//!
//! Default mode: any mismatch fails with a line-by-line diff.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

/// Samples from htmlx2jsx we skip. Originally 22 samples were skipped
/// as out-of-scope (pre-v0.2, no Svelte-4 support). Now that Svelte-4
/// surface features ship, the list is empty — kept as a seam so future
/// work can park individual samples pending triage without ripping out
/// the plumbing.
const SKIP_HTMLX: &[&str] = &[];

#[derive(Debug)]
struct Sample {
    /// Display name for diagnostics (e.g. `htmlx2jsx/each-block-basic`).
    name: String,
    /// Absolute path to the sample directory (contains `input.svelte`).
    input_dir: PathBuf,
    /// Absolute path to the snapshot file (`expected.emit.ts`).
    snapshot_path: PathBuf,
}

#[test]
fn emit_snapshots_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let snapshots_root = crate_dir.join("tests/emit_snapshots");

    let update = std::env::var("UPDATE_SNAPSHOTS").is_ok();

    let samples = collect_samples(&crate_dir, &snapshots_root);
    assert!(
        !samples.is_empty(),
        "no samples discovered. Did you forget `git submodule update --init --recursive`?"
    );

    let mut passed = 0usize;
    let mut created = 0usize;
    let mut updated = 0usize;
    let mut failures: Vec<(String, String)> = Vec::new();

    for sample in &samples {
        let actual = match run_emit(bin, &sample.input_dir) {
            Ok(out) => out,
            Err(err) => {
                failures.push((sample.name.clone(), format!("binary failed: {err}")));
                continue;
            }
        };

        let expected_exists = sample.snapshot_path.exists();
        let expected = if expected_exists {
            std::fs::read_to_string(&sample.snapshot_path).unwrap_or_default()
        } else {
            String::new()
        };

        if update {
            if let Some(parent) = sample.snapshot_path.parent() {
                std::fs::create_dir_all(parent).expect("create snapshot dir");
            }
            if actual != expected {
                std::fs::write(&sample.snapshot_path, &actual).expect("write snapshot");
                if expected_exists {
                    updated += 1;
                } else {
                    created += 1;
                }
            } else {
                passed += 1;
            }
        } else if !expected_exists {
            failures.push((
                sample.name.clone(),
                format!(
                    "no snapshot at {}. Run with UPDATE_SNAPSHOTS=1 to create one.",
                    sample.snapshot_path.display()
                ),
            ));
        } else if actual != expected {
            failures.push((sample.name.clone(), format_diff(&expected, &actual)));
        } else {
            passed += 1;
        }
    }

    if update {
        eprintln!(
            "emit_snapshots: {} passed, {} updated, {} created, {} failed (update mode)",
            passed,
            updated,
            created,
            failures.len()
        );
    } else {
        eprintln!(
            "emit_snapshots: {}/{} passed, {} failed",
            passed,
            samples.len(),
            failures.len()
        );
    }
    for (name, detail) in failures.iter().take(30) {
        eprintln!("\n--- FAIL {name} ---\n{detail}");
    }
    if failures.len() > 30 {
        eprintln!("\n... and {} more failures", failures.len() - 30);
    }

    assert!(
        failures.is_empty(),
        "emit_snapshots suite failed. Re-run with UPDATE_SNAPSHOTS=1 and inspect the diff to accept deliberate emit changes."
    );
}

fn collect_samples(crate_dir: &Path, snapshots_root: &Path) -> Vec<Sample> {
    let mut out = Vec::new();

    // Corpus 1: upstream svelte2tsx samples (filter to .v5).
    let v5_root = crate_dir
        .join("../../language-tools/packages/svelte2tsx/test/svelte2tsx/samples")
        .canonicalize()
        .ok();
    if let Some(root) = v5_root {
        for entry in read_dir_sorted(&root) {
            let name = entry
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if !name.ends_with(".v5") {
                continue;
            }
            if !entry.is_dir() {
                continue;
            }
            if !has_input_svelte(&entry) {
                continue;
            }
            out.push(Sample {
                name: format!("svelte2tsx_v5/{name}"),
                input_dir: entry,
                snapshot_path: snapshots_root
                    .join("svelte2tsx_v5")
                    .join(name)
                    .join("expected.emit.ts"),
            });
        }
    }

    // Corpus 2: upstream htmlx2jsx samples (filter out Svelte-4-only ones).
    let htmlx_root = crate_dir
        .join("../../language-tools/packages/svelte2tsx/test/htmlx2jsx/samples")
        .canonicalize()
        .ok();
    if let Some(root) = htmlx_root {
        for entry in read_dir_sorted(&root) {
            let name = entry
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if SKIP_HTMLX.contains(&name.as_str()) {
                continue;
            }
            if name.starts_with('_') || name.starts_with('.') {
                continue;
            }
            if !entry.is_dir() {
                continue;
            }
            if !has_input_svelte(&entry) {
                continue;
            }
            out.push(Sample {
                name: format!("htmlx2jsx/{name}"),
                input_dir: entry,
                snapshot_path: snapshots_root
                    .join("htmlx2jsx")
                    .join(name)
                    .join("expected.emit.ts"),
            });
        }
    }

    // Corpus 3: our own bug fixtures (any sample with an input.svelte).
    let bugs_root = crate_dir.join("../../fixtures/bugs").canonicalize().ok();
    if let Some(root) = bugs_root {
        for entry in read_dir_sorted(&root) {
            let name = entry
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("")
                .to_string();
            if name.starts_with('_') || name.starts_with('.') {
                continue;
            }
            if !entry.is_dir() {
                continue;
            }
            if !has_input_svelte(&entry) {
                continue;
            }
            out.push(Sample {
                name: format!("bugs/{name}"),
                input_dir: entry,
                snapshot_path: snapshots_root
                    .join("bugs")
                    .join(name)
                    .join("expected.emit.ts"),
            });
        }
    }

    out
}

fn read_dir_sorted(dir: &Path) -> Vec<PathBuf> {
    let mut entries: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .collect();
    entries.sort();
    entries
}

fn has_input_svelte(dir: &Path) -> bool {
    dir.join("input.svelte").is_file()
}

/// Run our binary with `--emit-ts` against a sample directory and
/// return the emitted TypeScript. The binary walks every `.svelte`
/// under the workspace, so a sample with auxiliary components
/// (e.g. `Inner.svelte` alongside `input.svelte`) gets every file's
/// emit concatenated with `// === <rel-path> ===` separators.
fn run_emit(bin: &str, input_dir: &Path) -> Result<String, String> {
    let out = Command::new(bin)
        .args([
            "--emit-ts",
            "--workspace",
            input_dir.to_str().ok_or("non-utf8 workspace path")?,
            "--no-tsconfig",
        ])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "exit {:?}\nstdout:\n{}\nstderr:\n{}",
            out.status.code(),
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Very simple line-based diff. Prints mismatched lines with context.
/// No attempt at LCS — just first-divergence + window.
fn format_diff(expected: &str, actual: &str) -> String {
    let exp_lines: Vec<&str> = expected.lines().collect();
    let act_lines: Vec<&str> = actual.lines().collect();
    let len = exp_lines.len().max(act_lines.len());
    let mut first_diff = None;
    for i in 0..len {
        if exp_lines.get(i) != act_lines.get(i) {
            first_diff = Some(i);
            break;
        }
    }
    let Some(at) = first_diff else {
        return "(no diff detected — whitespace or trailing-newline mismatch?)".to_string();
    };
    let ctx_start = at.saturating_sub(3);
    let ctx_end = (at + 8).min(len);
    let mut buf = format!("first divergence at line {}:\n", at + 1);
    for i in ctx_start..ctx_end {
        let e = exp_lines.get(i).copied().unwrap_or("<EOF>");
        let a = act_lines.get(i).copied().unwrap_or("<EOF>");
        if e == a {
            buf.push_str(&format!("  {:>4} | {}\n", i + 1, e));
        } else {
            buf.push_str(&format!("- {:>4} | {}\n", i + 1, e));
            buf.push_str(&format!("+ {:>4} | {}\n", i + 1, a));
        }
    }
    buf
}
