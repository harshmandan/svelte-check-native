//! `--svelte-warnings bridge` asks the project's own `svelte/compiler`
//! about every discovered component. The compiler's diagnostics are not
//! scoped by the tsconfig: svelte-check reports them for every file it
//! finds, as the native pass does. A monorepo root whose tsconfig
//! includes no components still gets its components' warnings.
//!
//! A mock compiler stands in for Svelte so the test pins our side: it
//! warns once on a component holding `MOCK_WARN`, and exits the process
//! on one holding `MOCK_CRASH`, so a bridge that stops answering is
//! seen.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

const MOCK_COMPILER: &str = r#"
exports.VERSION = '5.55.0';
exports.compile = (source) => {
    if (source.includes('MOCK_CRASH')) process.exit(3);
    const warnings = [];
    const at = source.indexOf('MOCK_WARN');
    if (at >= 0) {
        const before = source.slice(0, at).split('\n');
        const start = { line: before.length, column: before[before.length - 1].length };
        warnings.push({
            code: 'state_referenced_locally',
            message: 'mock warning',
            start,
            end: { line: start.line, column: start.column + 9 },
        });
    }
    return { warnings };
};
exports.parse = () => ({});
"#;

/// A workspace with the mock `svelte` package, a tsconfig that includes
/// only `src/`, and the given files.
fn workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp workspace");
    let root = dir.path();
    let svelte = root.join("node_modules/svelte");
    fs::create_dir_all(svelte.join("compiler")).unwrap();
    fs::write(
        svelte.join("package.json"),
        r#"{ "name": "svelte", "version": "5.55.0", "types": "./index.d.ts",
             "exports": { ".": { "types": "./index.d.ts" },
                          "./compiler": "./compiler/index.js" } }"#,
    )
    .unwrap();
    fs::write(svelte.join("compiler/index.js"), MOCK_COMPILER).unwrap();
    fs::write(
        svelte.join("index.d.ts"),
        "export type Component<Props = any, Exports = any, Bindings = any> =\n    \
         (internals: unknown, props: Props) => Exports;\n\
         export declare class SvelteComponent<Props = any, Events = any, Slots = any> {}\n",
    )
    .unwrap();
    fs::write(
        root.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext",
             "moduleResolution": "bundler", "strict": true, "skipLibCheck": true },
             "include": ["src/**/*"] }"#,
    )
    .unwrap();
    for (path, text) in files {
        let path = root.join(path);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }
    dir
}

/// The files the diagnostics name, the COMPLETED line, and stderr.
fn check(root: &Path) -> (Vec<String>, String, String) {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_dir.parent().and_then(Path::parent).unwrap();
    let tsgo = svn_typecheck::discover(repo_root)
        .expect("could not locate native TypeScript; install dependencies or set TSGO_BIN");
    let output = Command::new(env!("CARGO_BIN_EXE_svelte-check-native"))
        .args(["--workspace", root.to_str().unwrap()])
        .args(["--output", "machine-verbose"])
        .args(["--diagnostic-sources", "svelte"])
        .args(["--svelte-warnings", "bridge"])
        .env("TSGO_BIN", tsgo.path)
        .env("SVN_BRIDGE_WORKERS", "1")
        .output()
        .expect("run svelte-check-native");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut files = Vec::new();
    let mut completed = String::new();
    for line in stdout.lines() {
        let rest = line.split_once(' ').map_or("", |(_, rest)| rest);
        if rest.starts_with('{') {
            let d: serde_json::Value = serde_json::from_str(rest).unwrap();
            files.push(d["filename"].as_str().unwrap().to_string());
        } else if rest.starts_with("COMPLETED") {
            completed = rest.to_string();
        }
    }
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    (files, completed, stderr)
}

const WARNS: &str = "<script>\n  let a = 1;\n</script>\n<p>{a} MOCK_WARN</p>\n";

#[test]
fn components_outside_the_tsconfig_are_compiled() {
    let dir = workspace(&[("src/In.svelte", WARNS), ("other/Out.svelte", WARNS)]);
    let (files, completed, stderr) = check(dir.path());
    assert_eq!(
        files,
        ["other/Out.svelte", "src/In.svelte"],
        "stderr: {stderr}"
    );
    assert_eq!(
        completed,
        "COMPLETED 2 FILES 0 ERRORS 2 WARNINGS 2 FILES_WITH_PROBLEMS"
    );
}

/// A component the run leaves out (its template does not parse) is no
/// file of the run: nothing the compiler says about it is reported, and
/// it is not counted.
#[test]
fn a_component_left_out_of_the_run_is_not_compiled() {
    let unclosed = "<script>\n  let a = 1;\n</script>\n<div>{a} MOCK_WARN\n";
    let dir = workspace(&[("src/In.svelte", WARNS), ("other/Bad.svelte", unclosed)]);
    let (files, completed, stderr) = check(dir.path());
    assert_eq!(files, ["src/In.svelte"], "stderr: {stderr}");
    assert_eq!(
        completed,
        "COMPLETED 1 FILES 0 ERRORS 1 WARNINGS 1 FILES_WITH_PROBLEMS"
    );
}

#[test]
fn a_tsconfig_with_no_components_still_gets_their_warnings() {
    let dir = workspace(&[("apps/app/src/A.svelte", WARNS)]);
    let (files, completed, stderr) = check(dir.path());
    assert_eq!(files, ["apps/app/src/A.svelte"], "stderr: {stderr}");
    assert_eq!(
        completed,
        "COMPLETED 1 FILES 0 ERRORS 1 WARNINGS 1 FILES_WITH_PROBLEMS"
    );
}

#[test]
fn a_bridge_that_stops_answering_is_reported() {
    let dir = workspace(&[
        ("src/A.svelte", WARNS),
        ("src/B.svelte", "<p>MOCK_CRASH</p>\n"),
    ]);
    let (_, _, stderr) = check(dir.path());
    assert!(
        stderr.contains("svelte/compiler"),
        "a bridge failure must be reported on stderr, got: {stderr:?}"
    );
}
