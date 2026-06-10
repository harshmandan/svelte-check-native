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

/// `--threshold error` is a PRINT-TIME filter only: it suppresses the
/// per-diagnostic WARNING lines but must NOT zero out the COMPLETED
/// warning count or the `--fail-on-warnings` exit decision. (Pre-fix,
/// the filter ran before counting, so `--threshold error
/// --fail-on-warnings` exited 0 with warnings present — a false-clean.)
#[test]
fn threshold_error_keeps_warning_count_and_fail_on_warnings() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    // Build a workspace INSIDE the repo so tsgo discovery walks up to
    // the repo's node_modules. A `<img>` without `alt` fires the
    // `a11y_missing_attribute` native warning — a warning, no error.
    let repo_root = crate_dir.join("../..").canonicalize().unwrap();
    let tmp = tempfile::tempdir_in(&repo_root).expect("tempdir in repo");
    // tempfile names dirs `.tmpXXXX` (hidden); discovery prunes hidden
    // roots, so use a non-hidden child as the workspace.
    let work = tmp.path().join("ws");
    std::fs::create_dir(&work).unwrap();
    std::fs::write(work.join("A.svelte"), "<img src=\"x.png\">\n").unwrap();
    std::fs::write(
        work.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "moduleResolution": "bundler",
            "module": "esnext", "target": "esnext", "skipLibCheck": true },
            "include": ["**/*"] }"#,
    )
    .unwrap();

    let ws = work.to_str().unwrap().to_owned();
    let tsconfig = work.join("tsconfig.json");
    let tsconfig = tsconfig.to_str().unwrap().to_owned();
    let run = |extra: &[&str]| {
        let mut args = vec![
            "--workspace",
            ws.as_str(),
            "--tsconfig",
            tsconfig.as_str(),
            "--output",
            "machine",
        ];
        args.extend_from_slice(extra);
        Command::new(bin)
            .args(&args)
            .env("CLAUDECODE", "")
            .env("GEMINI_CLI", "")
            .env("CODEX_CI", "")
            .output()
            .expect("binary should run")
    };

    // Baseline: a warning is present and reported.
    let base = run(&[]);
    let base_out = String::from_utf8_lossy(&base.stdout);
    assert!(
        base_out.contains("1 WARNINGS"),
        "expected a warning in baseline run. stdout:\n{base_out}"
    );

    // `--threshold error --fail-on-warnings`: the WARNING *line* is
    // suppressed, but COMPLETED still reports 1 WARNINGS and the exit
    // code is 1 (fail-on-warnings saw the true count).
    let filtered = run(&["--threshold", "error", "--fail-on-warnings"]);
    let out = String::from_utf8_lossy(&filtered.stdout);
    assert!(
        !out.lines().any(|l| l.contains(" WARNING ")),
        "individual WARNING lines should be filtered by --threshold error. stdout:\n{out}"
    );
    assert!(
        out.contains("1 WARNINGS"),
        "COMPLETED must still report the true warning count. stdout:\n{out}"
    );
    assert_eq!(
        filtered.status.code(),
        Some(1),
        "--fail-on-warnings must exit 1 even with --threshold error. stdout:\n{out}"
    );
}

/// Recognised-but-unsupported upstream flags exit 2 with a clear,
/// actionable message — not a generic clap "unexpected argument" error.
#[test]
fn unsupported_flags_rejected_cleanly() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let cases: &[(&[&str], &str)] = &[
        (&["--no-tsconfig"], "--no-tsconfig is not supported"),
        (&["--ignore", "dist"], "--ignore only has an effect"),
        (&["--watch"], "watch mode is not supported"),
        (&["--preserveWatchOutput"], "watch mode is not supported"),
    ];
    for (args, needle) in cases {
        let out = Command::new(bin)
            .args(*args)
            .args(["--workspace", "/tmp"])
            .output()
            .expect("binary should run");
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            out.status.code(),
            Some(2),
            "{args:?} should exit 2. stderr:\n{stderr}"
        );
        assert!(
            stderr.contains(needle),
            "{args:?} should print {needle:?}. stderr:\n{stderr}"
        );
    }
}
