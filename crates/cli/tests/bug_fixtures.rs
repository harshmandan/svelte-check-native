//! Bug-fixtures integration suite.
//!
//! Spawns `node run.cjs` with env vars pointing at our binary and the
//! `fixtures/bugs/` directory. The runner iterates each fixture and asserts
//! on expected diagnostics. Same philosophy as the upstream-sanity suite:
//! Node is a thin harness; our binary is the system under test.
//!
//! The fixtures are split across several runner processes. Each fixture
//! spends nearly all its wall time waiting on a compiler subprocess, so a
//! single runner walking them in sequence leaves most of the machine
//! idle. They are independent — each owns its own directory and cache —
//! so this is purely a scheduling change.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

/// How many runner processes to split the fixtures across.
///
/// Each one drives a compiler subprocess that is itself threaded, so
/// this is deliberately below the core count: oversubscribing makes the
/// whole suite slower, not faster.
fn shard_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 4))
        .unwrap_or(1)
}

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

    let shards = shard_count();
    let results: Vec<_> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..shards)
            .map(|index| {
                let runner = runner.clone();
                let fixtures = fixtures.clone();
                scope.spawn(move || {
                    match Command::new("node")
                        .arg(runner.to_str().expect("runner path is utf-8"))
                        .env("SVELTE_CHECK_BIN", bin)
                        .env(
                            "FIXTURES_DIR",
                            fixtures.to_str().expect("fixtures dir is utf-8"),
                        )
                        .env("SHARD_COUNT", shards.to_string())
                        .env("SHARD_INDEX", index.to_string())
                        .output()
                    {
                        Ok(output) => output,
                        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                            panic!("`node` must be on PATH to run bug fixtures ({err})");
                        }
                        Err(err) => panic!("failed to spawn node: {err}"),
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

    // Sum the per-shard tallies so the assertion sees the whole corpus,
    // not one slice of it. A shard that fails to report a tally at all
    // (crash, truncated output) counts as a failure rather than being
    // read as zero.
    let mut total_passed = 0usize;
    let mut total_failed = 0usize;
    let mut missing_tally = false;

    for (index, output) in results.iter().enumerate() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        eprintln!("----- shard {index} stdout -----\n{stdout}");
        if !stderr.trim().is_empty() {
            eprintln!("----- shard {index} stderr -----\n{stderr}");
        }

        match parse_tally(&stdout) {
            Some((passed, failed)) => {
                total_passed += passed;
                total_failed += failed;
            }
            None => {
                missing_tally = true;
                eprintln!("shard {index} produced no tally line");
            }
        }
        if !output.status.success() {
            total_failed = total_failed.max(1);
        }
    }

    assert!(
        !missing_tally && total_failed == 0,
        "bug fixtures suite did not pass cleanly: {total_passed} passed, {total_failed} failed\
         {}",
        if missing_tally {
            " (a shard produced no tally — see its output above)"
        } else {
            ""
        }
    );
}

/// Pull `(passed, failed)` out of the runner's `N passed, M failed…` line.
fn parse_tally(stdout: &str) -> Option<(usize, usize)> {
    let line = stdout.lines().rev().find(|l| l.contains(" passed, "))?;
    let mut passed = None;
    let mut failed = None;
    for part in line.split(',') {
        let part = part.trim();
        if let Some(n) = part.strip_suffix(" passed") {
            passed = n.trim().parse().ok();
        } else if let Some(n) = part.strip_suffix(" failed") {
            failed = n.trim().parse().ok();
        }
    }
    Some((passed?, failed?))
}
