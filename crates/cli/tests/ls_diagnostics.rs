//! Language-server diagnostic-fixture parity suite.
//!
//! Drives `node run.cjs` against
//! `language-tools/packages/language-server/test/plugins/typescript/features/
//! diagnostics/fixtures/`, asserting that our binary's diagnostics match
//! upstream's `expectedv2.json` (or `expected_svelte_5.json` when present)
//! on `(file, line, character, code)` — the lossy-compare gate per
//! `notes/PARITY_TESTING_PLAN.md` P1.
//!
//! Same harness shape as `bug_fixtures.rs`, including the split across
//! several runner processes: the fixtures are independent and each
//! spends its wall time waiting on a compiler subprocess. Skip-list
//! (with reasons) is enforced inside the runner so the count stays
//! explicit, and stale-skip detection still covers every fixture — each
//! shard reports the stale skips it saw and the tallies are summed.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn ls_diagnostics_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let runner = crate_dir.join("tests/ls_diagnostics/run.cjs");
    assert!(runner.exists(), "runner not found at {}", runner.display());

    let fixtures_root = crate_dir
        .join("../../language-tools/packages/language-server/test/plugins/typescript/features/diagnostics/fixtures");
    let fixtures = match fixtures_root.canonicalize() {
        Ok(p) => p,
        Err(_) => {
            eprintln!(
                "SKIP: language-tools submodule fixtures not found at {}. \
                 Run `git submodule update --init --recursive` first.",
                fixtures_root.display()
            );
            return;
        }
    };

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
                            panic!("`node` must be on PATH to run LS diagnostic fixtures ({err})");
                        }
                        Err(err) => panic!("failed to spawn node: {err}"),
                    }
                })
            })
            .collect();
        handles.into_iter().map(|h| h.join().unwrap()).collect()
    });

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
        "LS diagnostic fixtures suite did not pass cleanly: \
         {total_passed} passed, {total_failed} failed{}",
        if missing_tally {
            " (a shard produced no tally — see its output above)"
        } else {
            ""
        }
    );
}

/// How many runner processes to split the fixtures across. Kept below
/// the core count because each drives a threaded compiler subprocess.
fn shard_count() -> usize {
    std::thread::available_parallelism()
        .map(|n| (n.get() / 2).clamp(1, 4))
        .unwrap_or(1)
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
        } else if let Some(rest) = part.split(" failed").next()
            && part.contains(" failed")
        {
            failed = rest.trim().parse().ok();
        }
    }
    Some((passed?, failed?))
}
