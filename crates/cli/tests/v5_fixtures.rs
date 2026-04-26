//! svelte2tsx `.v5` fixture parity suite.
//!
//! Walks the 63 Svelte-5-only fixtures from upstream svelte2tsx's test
//! corpus and asserts our binary produces zero tsgo errors against each.
//! Each fixture is a known-good Svelte 5 component so any error we
//! report is a real fidelity gap.
//!
//! Spawns `node run.cjs` with env vars locating the binary, the
//! samples directory inside the language-tools submodule, and our
//! shared base tsconfig. Same shim-adapter pattern as
//! upstream-sanity / bug-fixtures: Node is a thin harness, our binary
//! is the system under test.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn v5_fixtures_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let runner = crate_dir.join("tests/v5_fixtures/run.cjs");
    assert!(runner.exists(), "runner not found at {}", runner.display());

    let shim_tsconfig = crate_dir
        .join("tests/v5_fixtures/tsconfig.base.json")
        .canonicalize()
        .expect("base tsconfig must exist");

    let baselines = crate_dir
        .join("tests/v5_fixtures/baselines.json")
        .canonicalize()
        .expect("baselines.json must exist");

    let samples = crate_dir
        .join("../../language-tools/packages/svelte2tsx/test/svelte2tsx/samples")
        .canonicalize()
        .expect(
            "svelte2tsx samples not found. \
             Did you forget `git submodule update --init --recursive`?",
        );

    // Per-fixture workspaces live under /var/folders/... where there's
    // no enclosing node_modules to walk up to. Locate the local tsgo
    // ourselves and pass via TSGO_BIN so the binary doesn't fail in
    // discovery (which would silently inflate the pass-count).
    let tsgo = locate_local_tsgo(&crate_dir).expect(
        "could not locate the workspace's local tsgo install. \
         Run `npm install` at the repo root to install \
         @typescript/native-preview.",
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
            panic!("`node` must be on PATH to run the v5 fixtures suite ({err})");
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

    // Gate on the parity-pass count. Format:
    //   "v5 fixtures: 47/63 (40 clean, 7 within-baseline), 16 failed"
    // Lock the floor at the current baseline so regressions fail CI;
    // bump MIN_PASSED upward whenever we land more parity wins.
    //
    // Subprocess-failure path (no summary line) → 0/0 → MIN_PASSED
    // assertion fires immediately.
    let (passed, failed) = parse_summary(summary_line);
    const MIN_PASSED: usize = 47;
    const MAX_FAILED: usize = 16;
    assert!(
        passed >= MIN_PASSED,
        "v5 fixture pass count regressed: got {passed}, baseline is {MIN_PASSED}.\n\
         summary: {summary_line}\n\
         Either fix the regression or, if intentionally accepting a lower count, \
         lower MIN_PASSED in this test to match."
    );
    assert!(
        failed <= MAX_FAILED,
        "v5 fixture failure count regressed: got {failed}, baseline ceiling is \
         {MAX_FAILED}.\n\
         summary: {summary_line}\n\
         Investigate the new failures or, if expected, raise MAX_FAILED in this test."
    );
}

/// Parse the runner's summary line into `(passed, failed)`.
///
/// Format: `v5 fixtures: <PASS>/<TOTAL> (..., ...), <FAIL> failed`.
/// Robust to extra whitespace; returns `(0, usize::MAX)` on any parse
/// failure so a malformed/missing summary triggers the floor/ceiling
/// assertions above.
fn parse_summary(line: &str) -> (usize, usize) {
    // " 47/63" → split on "/" gets passed; "16 failed" → leading int.
    let after_colon = match line.split_once(':') {
        Some((_, rest)) => rest.trim(),
        None => return (0, usize::MAX),
    };
    let passed = after_colon
        .split('/')
        .next()
        .and_then(|s| s.trim().parse::<usize>().ok())
        .unwrap_or(0);
    let failed = after_colon
        .rsplit_once(',')
        .and_then(|(_, tail)| {
            tail.split_whitespace()
                .next()
                .and_then(|s| s.parse::<usize>().ok())
        })
        .unwrap_or(usize::MAX);
    (passed, failed)
}

/// Locate tsgo for these tests by delegating to the production
/// discovery layer. Per-fixture workspaces live under `/var/folders/`
/// where there's no enclosing `node_modules` to walk up to, so we run
/// discovery against the repo root and pass the result through to the
/// runner via TSGO_BIN.
///
/// The previous shape of this helper hard-coded a six-path list that
/// covered platform-native packages + the JS wrapper but missed
/// pnpm/bun package-store layouts (`.pnpm/@typescript+native-preview@…`)
/// — the production runtime supports those via
/// `svn_typecheck::discovery::find_in_package_store`. Reusing the
/// real discover() keeps test coverage aligned with shipping
/// behaviour and prevents tests from passing on a layout users can't
/// actually use.
fn locate_local_tsgo(crate_dir: &std::path::Path) -> Option<PathBuf> {
    let repo_root = crate_dir.parent()?.parent()?; // crates/cli → crates → repo
    svn_typecheck::discover(repo_root).ok().map(|b| b.path)
}
