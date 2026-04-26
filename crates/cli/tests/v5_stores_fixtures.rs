//! Local v5 store-pattern fixture suite.
//!
//! Re-uses the same Node runner as `v5_fixtures.rs` but points at our
//! locally-maintained `fixtures/v5-stores/` directory instead of the
//! upstream svelte2tsx corpus. These fixtures are direct ports of
//! upstream's store-pattern tests with Svelte 4 surface syntax
//! rewritten to Svelte 5 (`on:click=` → `onclick=`, etc.).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::{Path, PathBuf};
use std::process::Command;

#[test]
fn v5_stores_fixtures_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Re-use the upstream v5 runner — same shape, different fixture root.
    let runner = crate_dir.join("tests/v5_fixtures/run.cjs");
    assert!(runner.exists(), "runner not found at {}", runner.display());

    let shim_tsconfig = crate_dir
        .join("../../fixtures/v5-stores/_shared/tsconfig.base.json")
        .canonicalize()
        .expect("base tsconfig must exist");

    let baselines = crate_dir
        .join("tests/v5_stores_fixtures/baselines.json")
        .canonicalize()
        .expect("baselines.json must exist");

    let samples = crate_dir
        .join("../../fixtures/v5-stores")
        .canonicalize()
        .expect("fixtures/v5-stores must exist");

    let tsgo = locate_local_tsgo(&crate_dir).expect(
        "could not locate the workspace's local tsgo install. \
         Run `npm install` at the repo root.",
    );

    let output = match Command::new("node")
        .arg(&runner)
        .env("SVELTE_CHECK_BIN", bin)
        .env("SAMPLES_DIR", &samples)
        .env("SHIM_TSCONFIG", &shim_tsconfig)
        .env("BASELINES", &baselines)
        .env("TSGO_BIN", &tsgo)
        .output()
    {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            panic!("`node` must be on PATH ({err})");
        }
        Err(err) => panic!("failed to spawn node: {err}"),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("----- node stdout -----\n{stdout}");
    eprintln!("----- node stderr -----\n{stderr}");

    let summary_line = stdout
        .lines()
        .find(|l| l.starts_with("v5 fixtures:"))
        .unwrap_or("v5 fixtures: <no summary>");
    eprintln!("\n{summary_line}");

    let (passed, failed, skipped) = parse_summary(summary_line);
    const MIN_PASSED: usize = 18;
    const MAX_FAILED: usize = 6;
    assert!(
        passed >= MIN_PASSED,
        "v5-stores pass count regressed: got {passed}, baseline is {MIN_PASSED}.\n\
         summary: {summary_line}"
    );
    assert!(
        failed <= MAX_FAILED,
        "v5-stores failure count regressed: got {failed}, baseline ceiling is \
         {MAX_FAILED}.\n\
         summary: {summary_line}"
    );
    assert_eq!(
        skipped, 0,
        "v5-stores: {skipped} fixture(s) silently skipped — input-file heuristic \
         drift. summary: {summary_line}"
    );
}

fn parse_summary(line: &str) -> (usize, usize, usize) {
    let after_colon = match line.split_once(':') {
        Some((_, rest)) => rest.trim(),
        None => return (0, usize::MAX, usize::MAX),
    };
    let passed = after_colon
        .split('/')
        .next()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let failed = after_colon
        .split(',')
        .find_map(|seg| {
            seg.trim()
                .strip_suffix(" failed")
                .and_then(|n| n.trim().parse::<usize>().ok())
        })
        .unwrap_or(usize::MAX);
    let skipped = after_colon
        .split(',')
        .find_map(|seg| {
            seg.trim()
                .strip_suffix(" skipped")
                .and_then(|n| n.trim().parse::<usize>().ok())
        })
        .unwrap_or(0);
    (passed, failed, skipped)
}

/// Same delegation pattern as `v5_fixtures::locate_local_tsgo` —
/// reuse the production discover() so per-fixture workspaces in
/// /var/folders/ pick up tsgo via the same logic real users hit.
fn locate_local_tsgo(crate_dir: &Path) -> Option<PathBuf> {
    let repo_root = crate_dir.parent()?.parent()?;
    svn_typecheck::discover(repo_root).ok().map(|b| b.path)
}
