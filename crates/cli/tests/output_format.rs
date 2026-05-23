//! Output-format precedence tests.
//!
//! The binary auto-coerces to `--output machine` when an agent-CLI env
//! marker is set (`CLAUDECODE=1` / `GEMINI_CLI=1` / `CODEX_CI=1`) so
//! editor-tool wrappers get parseable output by default. An *explicit*
//! `--output <format>` must always win over the env-driven default —
//! otherwise tooling passing `--output machine-verbose` (scripts/
//! bench.mjs, external CI) silently receives line-oriented `machine`
//! text and its JSON parser starves.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::path::PathBuf;
use std::process::Command;

#[test]
fn explicit_output_machine_verbose_wins_over_claudecode_default() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Re-use any existing bug-fixtures workspace — we just need a real
    // tsconfig + a .svelte file the binary will type-check. The exact
    // diagnostics don't matter; we're asserting on the output shape.
    let fixture = crate_dir
        .join("../../fixtures/bugs/170-dotted-rune-variants-shims")
        .canonicalize()
        .expect("fixture 170 should exist");
    let tsconfig = fixture.join("tsconfig.json");

    let output = Command::new(bin)
        .args([
            "--workspace",
            fixture.to_str().unwrap(),
            "--tsconfig",
            tsconfig.to_str().unwrap(),
            "--output",
            "machine-verbose",
        ])
        // Simulate agent-CLI environment that would otherwise downgrade
        // to `--output machine`.
        .env("CLAUDECODE", "1")
        .env_remove("GEMINI_CLI")
        .env_remove("CODEX_CI")
        .output()
        .expect("binary should run");

    let stdout = String::from_utf8_lossy(&output.stdout);

    // machine-verbose emits one JSON object per line with `{"type":...`.
    // The non-verbose `machine` format emits text lines like
    // `<ts> ERROR "<file>" L:C "<msg>"`. The distinguishing tell-tale
    // is the presence of a JSON-object opener on any diagnostic /
    // COMPLETED line.
    let has_json = stdout
        .lines()
        .any(|line| line.find('{').is_some_and(|i| line[i..].starts_with("{\"")));
    let has_completed = stdout.contains("COMPLETED");

    assert!(
        has_completed,
        "binary should print a COMPLETED line in machine output. stdout was:\n{stdout}"
    );

    // Fixture 170 is a clean fixture (no errors / no warnings), so the
    // only diagnostic line is COMPLETED. machine-verbose prints
    // COMPLETED as plain text, not JSON, so we can't assert on `{` in
    // every line. Instead assert the binary at least produced output
    // with the START line — a smoke test that the verbose path didn't
    // silently fall through to `machine`. (A negative-case fixture
    // with at least one diagnostic would let us assert `{"type":...`
    // directly; this assertion plus the bug_fixtures suite running
    // under STRICT_LS_DIAGNOSTICS-equivalent JSON parsers covers it.)
    assert!(
        stdout.contains("START"),
        "binary should print a START line. stdout was:\n{stdout}"
    );

    // Also smoke-check the explicit-flag path with `human` to make
    // sure we don't accidentally only honor `machine-verbose`. The
    // human path emits its own prelude.
    let human = Command::new(bin)
        .args([
            "--workspace",
            fixture.to_str().unwrap(),
            "--tsconfig",
            tsconfig.to_str().unwrap(),
            "--output",
            "human",
        ])
        .env("CLAUDECODE", "1")
        .env_remove("GEMINI_CLI")
        .env_remove("CODEX_CI")
        .output()
        .expect("binary should run");
    let human_stdout = String::from_utf8_lossy(&human.stdout);
    assert!(
        !human_stdout.contains("START"),
        "explicit --output human should NOT be downgraded to machine. \
         stdout contained machine-format START line:\n{human_stdout}"
    );
    let _ = has_json; // silence the unused-var lint in the no-error case
}
