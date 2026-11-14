//! A script nested far deeper than anyone writes by hand (generated
//! types reach such depths) is checked like any other. The script
//! parser and the tree walks recurse once per level, so the check has
//! to run on a stack with room for it: a run that dies of a stack
//! overflow reports nothing at all.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

fn check(source: &str) -> (Option<i32>, String) {
    let dir = tempfile::tempdir().expect("temp workspace");
    let root = dir.path();
    fs::create_dir_all(root.join("src")).unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true, "noEmit": true }, "include": ["src"] }"#,
    )
    .unwrap();
    fs::write(root.join("src/Deep.svelte"), source).unwrap();
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_dir.parent().and_then(Path::parent).unwrap();
    let tsgo = svn_typecheck::discover(repo_root)
        .expect("could not locate native TypeScript; install dependencies or set TSGO_BIN");
    let output = Command::new(env!("CARGO_BIN_EXE_svelte-check-native"))
        .args(["--workspace", root.to_str().unwrap()])
        .args(["--output", "machine"])
        .env("TSGO_BIN", tsgo.path)
        .output()
        .expect("run svelte-check-native");
    (
        output.status.code(),
        String::from_utf8_lossy(&output.stdout).into_owned(),
    )
}

#[test]
fn a_deeply_nested_type_in_a_script_does_not_end_the_run() {
    let depth = 4_000;
    let nested = format!("{}number{}", "Array<".repeat(depth), ">".repeat(depth));
    let source = format!("<script lang=\"ts\">let x: {nested} = [];</script>\n{{x}}\n");
    let (code, stdout) = check(&source);
    // Killed by a signal, the process has no exit code.
    assert!(code.is_some(), "the run was killed: {stdout}");
    assert!(stdout.contains("COMPLETED 1 FILES"), "{stdout}");
}

#[test]
fn a_deeply_nested_expression_in_a_script_does_not_end_the_run() {
    let depth = 4_000;
    let nested = format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    let source = format!("<script>const x = {nested};</script>\n{{x}}\n");
    let (code, stdout) = check(&source);
    assert!(code.is_some(), "the run was killed: {stdout}");
    assert!(stdout.contains("COMPLETED"), "{stdout}");
}
