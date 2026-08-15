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

/// A scratch workspace created INSIDE the repo.
///
/// The compiler is discovered by walking up for `node_modules`, so a
/// workspace in the system temp dir has no engine to check with. These
/// tests used to dodge that by passing `--diagnostic-sources svelte`,
/// which skipped tsgo entirely — an idiom built on the very divergence
/// from upstream that has since been fixed, and one that made this
/// suite structurally unable to notice it. Rooting the scratch dir in
/// the repo lets the real engine run, the way every fixture suite does.
fn workspace_temp() -> tempfile::TempDir {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root");
    tempfile::Builder::new()
        .prefix("denominator-scratch-")
        .tempdir_in(repo_root)
        .expect("tempdir in repo")
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
    let ws = workspace_temp();
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

    // `--diagnostic-sources svelte` exercises the entries half of the
    // denominator. It no longer skips tsgo — TS diagnostics run on every
    // source selection, matching upstream — so the count below is the
    // real discovery count, not an artefact of a skipped phase.
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

#[test]
fn solution_escape_picks_first_reference_with_paths_and_reports_it() {
    // Documents (locks, without endorsing) the monorepo auto-escape's
    // pick-FIRST behavior: a project-references solution root with TWO
    // referenced apps redirects workspace + tsconfig to the first
    // reference whose extends chain declares `compilerOptions.paths`.
    // The second app is silently out of the run — its files don't
    // enter discovery or the `<N> FILES` denominator. The escape must
    // announce itself on stderr naming the chosen sub-project so a
    // two-app monorepo user can see which app was (and wasn't) checked.
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let ws = workspace_temp();
    let root = ws.path();

    write(
        &root.join("tsconfig.json"),
        r#"{
            "files": [],
            "references": [{ "path": "./app-a" }, { "path": "./app-b" }]
        }"#,
    );
    for app in ["app-a", "app-b"] {
        write(
            &root.join(app).join("tsconfig.json"),
            r#"{
                "compilerOptions": { "paths": { "$lib/*": ["./src/lib/*"] } },
                "include": ["src/**/*"]
            }"#,
        );
        write(
            &root.join(app).join("src/App.svelte"),
            "<script>let a = 1;</script><p>{a}</p>",
        );
    }

    // `--diagnostic-sources svelte` skips tsgo, keeping the test
    // hermetic; discovery + the denominator still exercise the
    // escaped workspace.
    let output = Command::new(bin)
        .args([
            "--workspace",
            root.to_str().unwrap(),
            "--output",
            "machine",
            "--diagnostic-sources",
            "svelte",
        ])
        .output()
        .expect("binary should run");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // The escape decision is announced on stderr, naming app-a.
    assert!(
        stderr.contains("redirected workspace to"),
        "escape must be visible on stderr. stderr:\n{stderr}"
    );
    let app_a = root.join("app-a");
    assert!(
        stderr.contains(app_a.to_str().unwrap()) || stderr.contains("app-a"),
        "stderr must name the chosen sub-project. stderr:\n{stderr}"
    );

    // Pick-first: only app-a's file is discovered/counted; app-b's
    // App.svelte is out of the run entirely.
    assert_eq!(
        completed_files(&stdout),
        Some(1),
        "only the first referenced app's files enter the denominator. stdout:\n{stdout}"
    );
}

/// Every `--diagnostic-sources` selection still runs the type checker.
///
/// On the command surface we mirror, upstream calls
/// `runTypeScriptDiagnostics` unconditionally and consults
/// `diagnosticSources` only for seeding svelte/css records
/// (`index.ts:444-510` vs `:324-330`). We used to gate the whole
/// emit->tsgo pipeline on the `js` source, so `--diagnostic-sources
/// svelte` reported zero type errors and exited 0 on a workspace full of
/// them — a run that checked nothing, indistinguishable from a clean one.
#[test]
fn type_errors_surface_for_every_diagnostic_source_selection() {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let ws = workspace_temp();
    let root = ws.path();

    write(
        &root.join("tsconfig.json"),
        r#"{ "extends": "../fixtures/bugs/_shared/tsconfig.base.json", "include": ["src/**/*"] }"#,
    );
    write(
        &root.join("src/bad.ts"),
        "export const x: number = \"not a number\";\n",
    );

    for sources in ["js", "svelte", "css", "ts"] {
        let output = Command::new(bin)
            .args([
                "--workspace",
                root.to_str().unwrap(),
                "--tsconfig",
                root.join("tsconfig.json").to_str().unwrap(),
                "--output",
                "machine",
                "--diagnostic-sources",
                sources,
            ])
            .output()
            .expect("binary should run");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            stdout.contains("ERROR"),
            "--diagnostic-sources {sources} reported no type error. stdout:\n{stdout}"
        );
    }
}
