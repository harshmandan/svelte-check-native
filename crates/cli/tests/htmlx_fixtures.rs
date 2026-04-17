//! htmlx2jsx template-control-flow parity suite.
//!
//! Walks the ~147 samples under upstream
//! `language-tools/packages/svelte2tsx/test/htmlx2jsx/samples/` that
//! exercise the template transformation layer — every `{#each}`,
//! `{#if}`, `{#await}`, `{#snippet}`, `{@const}`, `{@render}` shape
//! upstream tests — and runs each through our binary.
//!
//! These samples are **emit-shape tests** in upstream's suite (they
//! compare the generated TS string to a checked-in `expectedv2.js`).
//! For us they serve a different purpose: a rigid, upstream-maintained
//! corpus that exercises template-control-flow interactions our own
//! `{#each}`/`{#if}` grey-box fixtures can only cover one at a time.
//! When upstream adds or corrects a pattern, a submodule bump
//! surfaces the change here automatically.
//!
//! ### Sample filtering
//!
//! The upstream corpus includes Svelte 4 patterns we intentionally
//! drop (slots, `on:` directives, slot-let, etc.). The skip list
//! lives in `htmlx_fixtures/skip.json` and is consumed by the Node
//! runner.
//!
//! ### Pass criterion
//!
//! Each sample has a `max_errors` baseline in `baselines.json`. A run
//! passes if `errors ≤ max_errors`. Baselines start at the current
//! observed count — this suite's value is catching **regressions** as
//! we evolve emit, not proving clean-ness of every sample (many of
//! the samples reference undeclared identifiers by design, so tsgo
//! always reports `TS2304 Cannot find name` for them).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn htmlx_fixtures_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let runner = crate_dir.join("tests/htmlx_fixtures/run.cjs");
    assert!(runner.exists(), "runner not found at {}", runner.display());

    let shim_tsconfig = crate_dir
        .join("tests/htmlx_fixtures/tsconfig.base.json")
        .canonicalize()
        .expect("base tsconfig must exist");
    let baselines = crate_dir
        .join("tests/htmlx_fixtures/baselines.json")
        .canonicalize()
        .expect("baselines.json must exist");
    let skip = crate_dir
        .join("tests/htmlx_fixtures/skip.json")
        .canonicalize()
        .expect("skip.json must exist");
    let samples = crate_dir
        .join("../../language-tools/packages/svelte2tsx/test/htmlx2jsx/samples")
        .canonicalize()
        .expect(
            "htmlx2jsx samples not found. \
             Did you forget `git submodule update --init --recursive`?",
        );

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
        .env("SKIP_LIST", &skip)
        .env("TSGO_BIN", &tsgo)
        .output()
    {
        Ok(out) => out,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            panic!("`node` must be on PATH to run the htmlx fixtures suite ({err})");
        }
        Err(err) => panic!("failed to spawn node: {err}"),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("----- node stdout -----\n{stdout}");
    eprintln!("----- node stderr -----\n{stderr}");

    let summary_line = stdout
        .lines()
        .find(|l| l.starts_with("htmlx fixtures:"))
        .unwrap_or("htmlx fixtures: <no summary>");
    eprintln!("\n{summary_line}");

    assert!(
        output.status.success() && summary_line.contains("0 failed"),
        "htmlx fixtures regressed. exit: {:?}\nsummary: {summary_line}\n\
         If a baseline intentionally needs to move, bump the sample's \
         max_errors in baselines.json and document the reason.",
        output.status.code()
    );
}

fn locate_local_tsgo(crate_dir: &std::path::Path) -> Option<PathBuf> {
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
