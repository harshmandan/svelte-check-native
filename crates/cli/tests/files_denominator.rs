//! `<N> FILES` denominator parity with upstream svelte-check.
//!
//! Upstream's COMPLETED denominator is `|entries ∪ files-with-
//! diagnostics|` (index.ts `writeDiagnostics`, fed by
//! `getSvelteDiagnosticsForIncremental`), where `entries` is every
//! `.svelte` + Kit file discovered by `findFiles` WORKSPACE-WIDE —
//! only node_modules / dot-dir / `--ignore` filtering, no tsconfig
//! `include`/`exclude` scoping (incremental.ts `emitSvelteFiles`).
//! When `--diagnostic-sources` disables both `svelte` and `css`,
//! `getSvelteDiagnosticsForIncremental` returns no entry records at
//! all (index.ts early return), so the denominator collapses to just
//! the files that produced TS diagnostics.
//!
//! These tests lock both halves of that rule:
//!   1. A tsconfig whose `include` covers only a subtree must NOT
//!      shrink the denominator — out-of-scope `.svelte` files still
//!      count (they were discovered, and upstream counts them).
//!   2. A js-only invocation counts only diagnostic-bearing files.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create dir");
    }
    fs::write(path, content).expect("write fixture file");
}

/// Extract `N` from the machine-output `… COMPLETED N FILES …` line.
fn completed_files(stdout: &str) -> Option<u64> {
    let line = stdout.lines().find(|l| l.contains("COMPLETED"))?;
    let mut words = line.split_whitespace();
    while let Some(w) = words.next() {
        if w == "COMPLETED" {
            let n = words.next()?.parse().ok()?;
            assert_eq!(
                words.next(),
                Some("FILES"),
                "unexpected COMPLETED shape: {line}"
            );
            return Some(n);
        }
    }
    None
}

#[test]
fn scoped_tsconfig_include_does_not_shrink_the_denominator() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let ws = tempfile::tempdir().expect("tempdir");
    let root = ws.path();

    // Include covers only src/**; the .svelte file under other/ is
    // out of project scope but still discovered — upstream counts it.
    write(
        &root.join("tsconfig.json"),
        r#"{ "include": ["src/**/*"] }"#,
    );
    write(
        &root.join("src/App.svelte"),
        "<script>let a = 1;</script><p>{a}</p>",
    );
    write(
        &root.join("other/Out.svelte"),
        "<script>let b = 2;</script><p>{b}</p>",
    );
    // A Kit route file — upstream's findFiles counts kit files too.
    write(
        &root.join("src/routes/+page.ts"),
        "export function load() { return {}; }\n",
    );

    // `--diagnostic-sources svelte` skips tsgo entirely, so this runs
    // hermetically (no node_modules needed) while still exercising the
    // entries half of the denominator.
    let output = Command::new(bin)
        .args([
            "--workspace",
            root.to_str().unwrap(),
            "--tsconfig",
            root.join("tsconfig.json").to_str().unwrap(),
            "--output",
            "machine",
            "--diagnostic-sources",
            "svelte",
        ])
        .output()
        .expect("binary should run");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // 2 .svelte files (one in scope, one out) + 1 kit file.
    assert_eq!(
        completed_files(&stdout),
        Some(3),
        "expected the workspace-wide discovery count. stdout:\n{stdout}"
    );
}

#[test]
fn js_only_sources_count_only_files_with_diagnostics() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // Clean fixture (expected.json: clean) with a real tsconfig and a
    // repo-root tsgo install reachable via node_modules walk-up.
    let fixture = crate_dir
        .join("../../fixtures/bugs/170-dotted-rune-variants-shims")
        .canonicalize()
        .expect("fixture 170 should exist");

    let output = Command::new(bin)
        .args([
            "--workspace",
            fixture.to_str().unwrap(),
            "--tsconfig",
            fixture.join("tsconfig.json").to_str().unwrap(),
            "--output",
            "machine",
            "--diagnostic-sources",
            "js",
        ])
        .output()
        .expect("binary should run");
    let stdout = String::from_utf8_lossy(&output.stdout);

    // No svelte/css sources → upstream produces no per-entry records;
    // the fixture is clean, so no file carries a diagnostic either.
    assert_eq!(
        completed_files(&stdout),
        Some(0),
        "js-only runs count only diagnostic-bearing files. stdout:\n{stdout}"
    );
}
