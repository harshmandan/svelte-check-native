//! `SVN_ENGINE` picks the compiler family. TypeScript is the default
//! and tsrs is only ever used when asked for, even when both are
//! installed. A missing engine fails the check the way a missing
//! TypeScript does: the error, `svelte-check failed`, exit 1. Stand-in compilers (shell scripts at each engine's
//! platform-package path) report which one ran.

#![cfg(unix)]
#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::process::{Command, Output};

/// The host's platform-package directory names for TypeScript and tsrs,
/// or `None` where either publishes no binary.
fn platform_dirs() -> Option<(&'static str, &'static str)> {
    match (std::env::consts::OS, std::env::consts::ARCH) {
        ("macos", "aarch64") => Some(("typescript-darwin-arm64", "darwin-arm64")),
        ("linux", "x86_64") => Some(("typescript-linux-x64", "linux-x64-gnu")),
        ("linux", "aarch64") => Some(("typescript-linux-arm64", "linux-arm64-gnu")),
        _ => None,
    }
}

/// A compiler that reports one error on `src/a.ts`, naming itself.
fn stand_in(path: &Path, name: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(
        path,
        format!("#!/bin/sh\necho \"src/a.ts:1:7 - error TS2322: checked by {name}\"\nexit 2\n"),
    )
    .unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn workspace(dir: &Path, typescript: bool, tsrs: bool) -> Option<()> {
    let (ts_dir, tsrs_dir) = platform_dirs()?;
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "strict": true }, "include": ["src"] }"#,
    )
    .unwrap();
    std::fs::write(dir.join("src/a.ts"), "const a: number = 1;\n").unwrap();
    if typescript {
        stand_in(
            &dir.join(format!("node_modules/@typescript/{ts_dir}/lib/tsc")),
            "typescript",
        );
    }
    if tsrs {
        stand_in(
            &dir.join(format!("node_modules/@tsrs/{tsrs_dir}/tsrs")),
            "tsrs",
        );
    }
    Some(())
}

fn run(ws: &Path, engine: Option<&str>) -> Output {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_svelte-check-native"));
    cmd.args(["--workspace", ws.to_str().unwrap(), "--output", "machine"])
        .env_remove("TSGO_BIN")
        .env_remove("SVN_ENGINE")
        .env("SVN_DISABLE_REPLAY", "1");
    if let Some(engine) = engine {
        cmd.env("SVN_ENGINE", engine);
    }
    cmd.output().expect("binary should run")
}

fn stdout(out: &Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn typescript_is_used_by_default_when_both_are_installed() {
    let tmp = tempfile::tempdir().unwrap();
    if workspace(tmp.path(), true, true).is_none() {
        return;
    }
    let out = run(tmp.path(), None);
    assert!(stdout(&out).contains("checked by typescript"), "{out:?}");
    assert!(!stdout(&out).contains("checked by tsrs"), "{out:?}");
}

#[test]
fn tsrs_is_used_when_asked_for() {
    let tmp = tempfile::tempdir().unwrap();
    if workspace(tmp.path(), true, true).is_none() {
        return;
    }
    let out = run(tmp.path(), Some("tsrs"));
    assert!(stdout(&out).contains("checked by tsrs"), "{out:?}");
    assert!(!stdout(&out).contains("checked by typescript"), "{out:?}");
}

#[test]
fn asking_for_a_missing_tsrs_fails_like_a_missing_typescript() {
    let tmp = tempfile::tempdir().unwrap();
    if workspace(tmp.path(), true, false).is_none() {
        return;
    }
    let out = run(tmp.path(), Some("tsrs"));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(
        stderr.trim_end().ends_with("svelte-check failed"),
        "stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("tsrs is not installed"),
        "stderr:\n{stderr}"
    );
    assert!(!stdout(&out).contains("checked by typescript"), "{out:?}");
}

#[test]
fn tsrs_alone_is_not_used_by_default() {
    let tmp = tempfile::tempdir().unwrap();
    if workspace(tmp.path(), false, true).is_none() {
        return;
    }
    let out = run(tmp.path(), None);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(1), "stderr:\n{stderr}");
    assert!(
        stderr.trim_end().ends_with("svelte-check failed"),
        "stderr:\n{stderr}"
    );
    assert!(stderr.contains("SVN_ENGINE=tsrs"), "stderr:\n{stderr}");
    assert!(!stdout(&out).contains("checked by tsrs"), "{out:?}");
}
