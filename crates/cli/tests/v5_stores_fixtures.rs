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
}

fn locate_local_tsgo(crate_dir: &Path) -> Option<PathBuf> {
    let repo_root = crate_dir.parent()?.parent()?;
    [
        repo_root.join("node_modules/@typescript/native-preview-darwin-arm64/lib/tsgo"),
        repo_root.join("node_modules/@typescript/native-preview-darwin-x64/lib/tsgo"),
        repo_root.join("node_modules/@typescript/native-preview-linux-arm64/lib/tsgo"),
        repo_root.join("node_modules/@typescript/native-preview-linux-x64/lib/tsgo"),
        repo_root.join("node_modules/@typescript/native-preview-win32-x64/lib/tsgo.exe"),
        repo_root.join("node_modules/@typescript/native-preview/bin/tsgo.js"),
    ]
    .into_iter()
    .find(|p| p.is_file())
}
