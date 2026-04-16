//! Upstream `svelte-check` sanity suite, run unmodified against our binary.
//!
//! This is NOT a port. We spawn `node --require <shim.cjs> <upstream-test-sanity.js>`
//! with `SVELTE_CHECK_BIN` pointing at our Rust binary. The shim monkey-patches
//! `child_process.execFileSync` so upstream's `execFileSync('node', [CLI, ...args])`
//! gets redirected to `execFileSync(OUR_BIN, [...args])`. Upstream's test file
//! runs byte-for-byte unmodified.
//!
//! Advantages of this approach over porting:
//! - Zero expected-error arrays duplicated in our tree.
//! - Submodule bump = upstream test update applied for free.
//! - Upstream test is the definition of "correct" — we can't drift.

// Tests are allowed to panic loudly on setup failures.
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn upstream_sanity_suite() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    let shim = crate_dir.join("tests/upstream_sanity/shim.cjs");
    assert!(shim.exists(), "shim not found at {}", shim.display());

    let test_script = crate_dir
        .join("../../language-tools/packages/svelte-check/test-sanity.js")
        .canonicalize()
        .expect(
            "upstream test-sanity.js not found. \
             Did you forget `git submodule update --init --recursive`?",
        );

    // test-sanity.js uses `cwd: __dirname` in its own execFileSync calls and
    // resolves `./test-success` / `./test-error` relative to its own dir. Our
    // spawn cwd matches that dir so everything lines up.
    let cwd = test_script
        .parent()
        .expect("test-sanity.js must have a parent dir");

    let output = match Command::new("node")
        .args([
            "--require",
            shim.to_str().expect("shim path must be utf-8"),
            test_script.to_str().expect("test path must be utf-8"),
        ])
        .env("SVELTE_CHECK_BIN", bin)
        .current_dir(cwd)
        .output()
    {
        Ok(output) => output,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
            panic!("`node` must be on PATH to run the upstream parity suite ({err})");
        }
        Err(err) => panic!("failed to spawn node: {err}"),
    };

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Surface output when running with `cargo test -- --nocapture`.
    eprintln!("----- node stdout -----\n{stdout}");
    eprintln!("----- node stderr -----\n{stderr}");

    let tail = stdout.lines().last().unwrap_or("<no output>");
    let tail_is_clean = tail.contains("0 failed");

    assert!(
        output.status.success() && tail_is_clean,
        "upstream sanity suite did not pass cleanly.\n\
         exit code: {:?}\n\
         tail line: {tail}",
        output.status.code()
    );
}
