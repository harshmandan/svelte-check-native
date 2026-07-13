//! End-to-end behavior of `svelte.config.js` settings on the native
//! lint pass.
//!
//! Upstream compiles every component with the config's
//! `compilerOptions` (SvelteDocument.getCompiled passes them into
//! `svelte.compile`), and resolves the config PER DOCUMENT via upward
//! search from each file's directory (Document.ts →
//! configLoader.awaitConfig → searchConfigPathUpwards) — nearest
//! config wins, no merging. These tests drive the real binary with
//! `--diagnostic-sources svelte` (no tsgo needed) and assert on the
//! machine-output diagnostics.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("create dir");
    }
    fs::write(path, content).expect("write fixture file");
}

fn run_svelte_only(workspace: &Path) -> String {
    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let output = Command::new(bin)
        .args([
            "--workspace",
            workspace.to_str().unwrap(),
            "--tsconfig",
            workspace.join("tsconfig.json").to_str().unwrap(),
            "--output",
            "machine",
            "--diagnostic-sources",
            "svelte",
        ])
        .output()
        .expect("binary should run");
    String::from_utf8_lossy(&output.stdout).into_owned()
}

/// `compilerOptions.runes: true` in the config forces runes mode for a
/// component that auto-detection would classify as legacy, so
/// runes-only lints (here: `slot_element_deprecated` on `<slot>`) fire
/// exactly like upstream's config-driven compile.
#[test]
fn config_runes_true_forces_runes_mode_lints() {
    // Legacy-looking component: no rune calls, plus a <slot> element.
    let component = "<script>let a = 1;</script><p>{a}</p><slot></slot>";

    // Control: no config → auto-detect says legacy → no deprecation.
    let ws = tempfile::tempdir().expect("tempdir");
    write(&ws.path().join("tsconfig.json"), "{}");
    write(&ws.path().join("src/App.svelte"), component);
    let stdout = run_svelte_only(ws.path());
    assert!(
        !stdout.contains("slot_element_deprecated"),
        "auto-detected legacy component must not fire runes-only lints. stdout:\n{stdout}"
    );

    // With `runes: true` the same component is compiled in runes mode.
    let ws = tempfile::tempdir().expect("tempdir");
    write(&ws.path().join("tsconfig.json"), "{}");
    write(
        &ws.path().join("svelte.config.js"),
        "export default { compilerOptions: { runes: true } };",
    );
    write(&ws.path().join("src/App.svelte"), component);
    let stdout = run_svelte_only(ws.path());
    assert!(
        stdout.contains("slot_element_deprecated"),
        "config-forced runes mode must fire runes-only lints. stdout:\n{stdout}"
    );
}

/// `compilerOptions.runes: false` pins legacy mode even when the file
/// would auto-detect as… still legacy; the interesting half is that a
/// rune-calling file with `runes: false` upstream is a compile error —
/// we don't model that, but forced-false must at minimum not ENABLE
/// runes-only lints.
#[test]
fn config_runes_false_keeps_legacy_mode_lints_off() {
    let ws = tempfile::tempdir().expect("tempdir");
    write(&ws.path().join("tsconfig.json"), "{}");
    write(
        &ws.path().join("svelte.config.js"),
        "export default { compilerOptions: { runes: false } };",
    );
    write(
        &ws.path().join("src/App.svelte"),
        "<script>let a = 1;</script><p>{a}</p><slot></slot>",
    );
    let stdout = run_svelte_only(ws.path());
    assert!(
        !stdout.contains("slot_element_deprecated"),
        "runes: false must not enable runes-only lints. stdout:\n{stdout}"
    );
}
