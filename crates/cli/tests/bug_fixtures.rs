//! Bug-fixtures integration suite.
//!
//! Spawns `node run.cjs` with env vars pointing at our binary and the
//! `fixtures/bugs/` directory. The runner iterates each fixture and asserts
//! on expected diagnostics. Same philosophy as the upstream-sanity suite:
//! Node is a thin harness; our binary is the system under test.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn bug_fixtures_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let runner = crate_dir.join("tests/bug_fixtures/run.cjs");
    assert!(runner.exists(), "runner not found at {}", runner.display());

    let fixtures = crate_dir
        .join("../../fixtures/bugs")
        .canonicalize()
        .expect("fixtures/bugs/ not found — has it been created yet?");

    let output = match Command::new("node")
        .arg(runner.to_str().expect("runner path is utf-8"))
        .env("SVELTE_CHECK_BIN", bin)
        .env(
            "FIXTURES_DIR",
            fixtures.to_str().expect("fixtures dir is utf-8"),
        )
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            panic!("`node` must be on PATH to run bug fixtures ({err})");
        }
        Err(err) => panic!("failed to spawn node: {err}"),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    eprintln!("----- node stdout -----\n{stdout}");
    eprintln!("----- node stderr -----\n{stderr}");

    let tail = stdout.lines().last().unwrap_or("<no output>");

    assert!(
        output.status.success() && tail.contains("0 failed"),
        "bug fixtures suite did not pass cleanly.\n\
         exit: {:?}\n\
         tail: {tail}",
        output.status.code()
    );
}
