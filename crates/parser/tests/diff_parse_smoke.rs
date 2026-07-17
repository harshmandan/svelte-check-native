//! Differential-parse harness smoke test.
//!
//! Runs `scripts/diff-parse.mjs` (real `svelte/compiler` `parse()` vs our
//! parser, normalized skeleton diff) over a handful of small fixtures and
//! asserts they come out IDENTICAL. This keeps the harness itself from
//! rotting — the bench-wide sweep is interactive tooling, not a test
//! (bench/ isn't part of `cargo test`), but the tool must always run.
//!
//! Skips cleanly when no reference svelte install is available (the
//! script resolves one from the target's workspace or any bench/*
//! install — both dev-local).

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn diff_parse_smoke() {
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_dir.join("../..").canonicalize().expect("repo root");
    let script = repo_root.join("scripts/diff-parse.mjs");
    assert!(script.exists(), "script not found at {}", script.display());

    // `cargo test -p svn-parser` builds the crate's examples; the test
    // binary sits in target/<profile>/deps, the example one level up in
    // target/<profile>/examples. Pass it explicitly so the script can't
    // pick a stale binary from another profile.
    let test_exe = std::env::current_exe().expect("test exe path");
    let dump_bin = test_exe
        .parent()
        .and_then(|deps| deps.parent())
        .map(|profile| profile.join("examples/dump_parse"))
        .expect("derive examples dir from test exe path");
    assert!(
        dump_bin.exists(),
        "dump_parse example not built at {}",
        dump_bin.display()
    );

    for fixture in ["basic.svelte", "blocks.svelte", "rawtext.svelte"] {
        let path = crate_dir.join("tests/diff_parse_smoke").join(fixture);
        assert!(path.exists(), "fixture missing: {}", path.display());

        let output = match Command::new("node")
            .arg(&script)
            .arg(&path)
            .arg("--dump-bin")
            .arg(&dump_bin)
            .current_dir(&repo_root)
            .output()
        {
            Ok(output) => output,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                panic!("`node` must be on PATH to run the diff-parse smoke test ({err})");
            }
            Err(err) => panic!("failed to spawn node: {err}"),
        };

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        // No reference compiler on this machine (no bench/ checkouts):
        // the harness can't run; skip rather than fail.
        if stderr.contains("No svelte install found") {
            eprintln!("SKIP: no svelte install available for diff-parse smoke test");
            return;
        }

        assert!(
            output.status.success() && stdout.contains("IDENTICAL"),
            "diff-parse smoke failed on {fixture}\nexit: {:?}\n--- stdout ---\n{stdout}\n--- stderr ---\n{stderr}",
            output.status.code()
        );
    }
}
