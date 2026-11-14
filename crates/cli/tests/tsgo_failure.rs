//! A compiler run that does not complete must fail the check, the way
//! svelte-check fails it: the error and `svelte-check failed` on stderr,
//! nothing on stdout, exit 1. A stand-in compiler (a shell script passed
//! as `TSGO_BIN`) plays the crash.

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

fn workspace(dir: &Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true }, "include": ["src"] }"#,
    )
    .unwrap();
    std::fs::write(
        dir.join("src/A.svelte"),
        "<script lang=\"ts\">let a: number = 1;</script>\n",
    )
    .unwrap();
}

fn run_with_compiler(script: &str) -> Output {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    workspace(&ws);
    let compiler = tmp.path().join("tsc");
    std::fs::write(&compiler, script).unwrap();
    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    Command::new(env!("CARGO_BIN_EXE_svelte-check-native"))
        .args(["--workspace", ws.to_str().unwrap(), "--output", "machine"])
        .env("TSGO_BIN", &compiler)
        .env("SVN_DISABLE_REPLAY", "1")
        .output()
        .expect("binary should run")
}

fn assert_failed(out: &Output, needle: &str) {
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(stdout.is_empty(), "nothing on stdout, got:\n{stdout}");
    assert!(stderr.contains(needle), "stderr:\n{stderr}");
    assert!(
        stderr.trim_end().ends_with("svelte-check failed"),
        "stderr:\n{stderr}"
    );
}

#[test]
fn nonzero_exit_without_a_diagnostic_fails() {
    let out = run_with_compiler("#!/bin/sh\necho 'panic: index out of range' >&2\nexit 2\n");
    assert_failed(&out, "exited with code 2 without a parseable diagnostic");
    // What the compiler printed is the only explanation, so it is shown.
    assert!(String::from_utf8_lossy(&out.stderr).contains("panic: index out of range"));
}

#[test]
fn killed_compiler_fails() {
    let out = run_with_compiler("#!/bin/sh\nkill -9 $$\n");
    assert_failed(&out, "was killed by signal SIGKILL");
}

/// A run that fails early still finishes what it started beside the
/// type check: TypeScript's transpile of the components' scripts runs in
/// a process of its own and writes into a temporary directory. Exiting
/// without waiting would leave that process running and its directory
/// behind.
#[test]
fn a_failed_run_waits_for_the_transpile_it_started() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("ws");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    std::fs::write(
        ws.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true }, "include": ["src"] }"#,
    )
    .unwrap();
    // An enum is printed as other code, so the component is linted from
    // TypeScript's real transpile.
    std::fs::write(
        ws.join("src/A.svelte"),
        "<script lang=\"ts\">enum E { A } let a: number = E.A;</script>\n<p>{a}</p>\n",
    )
    .unwrap();
    let ts = ws.join("node_modules/typescript");
    std::fs::create_dir_all(&ts).unwrap();
    std::fs::write(
        ts.join("package.json"),
        r#"{ "name": "typescript", "version": "5.9.0", "main": "index.js" }"#,
    )
    .unwrap();
    std::fs::write(
        ts.join("index.js"),
        r#"exports.ModuleKind = { ESNext: 99 };
exports.ScriptTarget = { ESNext: 99 };
exports.transpileModule = (content) => {
    const end = Date.now() + 1500;
    while (Date.now() < end) {}
    require('fs').writeFileSync(require('path').join(__dirname, 'done'), '');
    return { outputText: content, sourceMapText: '{"mappings":"AAAA"}' };
};
"#,
    )
    .unwrap();
    let compiler = tmp.path().join("tsc");
    std::fs::write(&compiler, "#!/bin/sh\nexit 2\n").unwrap();
    std::fs::set_permissions(&compiler, std::fs::Permissions::from_mode(0o755)).unwrap();
    let scratch = tmp.path().join("tmp");
    std::fs::create_dir_all(&scratch).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_svelte-check-native"))
        .args(["--workspace", ws.to_str().unwrap(), "--output", "machine"])
        .env("TSGO_BIN", &compiler)
        .env("TMPDIR", &scratch)
        .env("SVN_DISABLE_REPLAY", "1")
        .output()
        .expect("binary should run");
    assert_failed(&out, "exited with code 2 without a parseable diagnostic");
    assert!(ts.join("done").is_file(), "the transpile was left running");
    let left: Vec<_> = std::fs::read_dir(&scratch)
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}
