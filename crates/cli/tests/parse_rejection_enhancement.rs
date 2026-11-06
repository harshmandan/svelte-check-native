//! Components svelte2tsx can't convert, reported the way the default
//! svelte-check engine reports them (a TSGO-ENHANCEMENT; `--tsgo` drops
//! them silently).
//!
//! The errors come from the project's own `svelte/compiler` through the
//! Node bridge. A mock compiler stands in for it here so the test pins
//! our side — which files are asked about, how the two errors are shaped,
//! suppressed and counted — independent of any Svelte release. The mock
//! throws Svelte-shaped errors: `compile()` and `parse()` each throw at
//! the first `<div>` that is never closed, or at a stray `{/if}`, unless
//! the source opts out with a marker comment.

#![allow(clippy::expect_used, clippy::unwrap_used)]

use std::fs;
use std::path::Path;
use std::process::Command;

const MOCK_COMPILER: &str = r#"
function fail(code, message, source, needle) {
    const e = new Error(message + '\nhttps://svelte.dev/e/' + code);
    e.code = code;
    if (needle !== undefined) {
        const at = source.indexOf(needle);
        const before = source.slice(0, at).split('\n');
        e.start = { line: before.length, column: before[before.length - 1].length };
        e.end = { line: e.start.line, column: e.start.column + 1 };
    }
    throw e;
}
function reject(source) {
    if (source.includes('{/if}')) {
        fail('block_unexpected_close', 'Unexpected block closing tag', source, '{/if}');
    }
    if (source.includes('<div>') && !source.includes('</div>')) {
        fail('element_unclosed', '`<div>` was left open', source, '<div>');
    }
}
exports.VERSION = '5.55.0';
exports.compile = (source, options) => {
    if (options.dev !== undefined && typeof options.dev !== 'boolean') {
        fail('options_invalid_value', 'Invalid compiler option: dev should be true or false, if specified');
    }
    if (!source.includes('MOCK_COMPILE_OK')) reject(source);
    return { warnings: [] };
};
exports.parse = (source) => {
    if (!source.includes('MOCK_PARSE_OK')) reject(source);
    return {};
};
"#;

/// A workspace with the mock `svelte` package and the given files.
fn workspace(files: &[(&str, &str)]) -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp workspace");
    let root = dir.path();
    let svelte = root.join("node_modules/svelte");
    fs::create_dir_all(svelte.join("compiler")).unwrap();
    fs::write(
        svelte.join("package.json"),
        r#"{ "name": "svelte", "version": "5.55.0", "types": "./index.d.ts",
             "exports": { ".": { "types": "./index.d.ts" },
                          "./elements": { "types": "./elements.d.ts" },
                          "./compiler": "./compiler/index.js" } }"#,
    )
    .unwrap();
    fs::write(svelte.join("compiler/index.js"), MOCK_COMPILER).unwrap();
    fs::write(
        svelte.join("index.d.ts"),
        "export type Component<Props = any, Exports = any, Bindings = any> =\n    \
         (internals: unknown, props: Props) => Exports;\n\
         export interface ComponentConstructorOptions<Props = Record<string, any>> {\n    \
         target: any;\n    props?: Props;\n}\n\
         export declare class SvelteComponent<Props = any, Events = any, Slots = any> {}\n\
         export type Snippet<Parameters extends unknown[] = []> = (...args: Parameters) => unknown;\n",
    )
    .unwrap();
    fs::write(
        svelte.join("elements.d.ts"),
        "export interface HTMLAttributes<T> { [name: string]: any; }\n\
         export interface SvelteHTMLElements { [name: string]: HTMLAttributes<any>; }\n",
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

/// The machine-verbose diagnostics (timestamps stripped) and the
/// COMPLETED line.
fn check(root: &Path, extra: &[&str]) -> (Vec<serde_json::Value>, String) {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_dir.parent().and_then(Path::parent).unwrap();
    let tsgo = svn_typecheck::discover(repo_root)
        .expect("could not locate native TypeScript; install dependencies or set TSGO_BIN");
    let output = Command::new(env!("CARGO_BIN_EXE_svelte-check-native"))
        .args(["--workspace", root.to_str().unwrap()])
        .args(["--output", "machine-verbose"])
        .args(extra)
        .env("TSGO_BIN", tsgo.path)
        .output()
        .expect("run svelte-check-native");
    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut diagnostics = Vec::new();
    let mut completed = String::new();
    for line in stdout.lines() {
        let rest = line.split_once(' ').map_or("", |(_, rest)| rest);
        if rest.starts_with('{') {
            diagnostics.push(serde_json::from_str(rest).unwrap());
        } else if rest.starts_with("COMPLETED") {
            completed = rest.to_string();
        }
    }
    (diagnostics, completed)
}

const OK: &str = "<script lang=\"ts\">\n  let n: number = 1;\n</script>\n<p>{n}</p>\n";
const UNCLOSED: &str = "<script>\n  let a = 1;\n</script>\n<div>{a}\n";
const STRAY_CLOSE_TS: &str = "<script lang=\"ts\">\n  let a = 1;\n</script>\n{#if a}<div>{/if}\n";
const PARSES: &str = "<!-- MOCK_PARSE_OK -->\n<div>\n";

#[test]
fn rejected_components_get_both_errors_and_count() {
    let dir = workspace(&[
        ("src/Ok.svelte", OK),
        ("src/Unclosed.svelte", UNCLOSED),
        ("src/StrayClose.svelte", STRAY_CLOSE_TS),
        ("src/Parses.svelte", PARSES),
    ]);
    let (diagnostics, completed) = check(dir.path(), &[]);
    let unclosed_at = serde_json::json!({ "line": 3, "character": 0 });
    let unclosed_to = serde_json::json!({ "line": 3, "character": 1 });
    let unclosed_message = "`<div>` was left open\nhttps://svelte.dev/e/element_unclosed";
    // Files in discovery order.
    assert_eq!(
        diagnostics,
        vec![
            // "Unexpected …" contains "expected": with a `lang` attribute
            // and no `preprocess` of the project's own, the compiler's
            // error is dropped; svelte2tsx's never is.
            serde_json::json!({
                "type": "ERROR", "filename": "src/StrayClose.svelte",
                "start": { "line": 3, "character": 12 },
                "end": { "line": 3, "character": 13 },
                "message": "Unexpected block closing tag\nhttps://svelte.dev/e/block_unexpected_close",
                "code": -1, "source": "ts",
            }),
            // The compiler's error first, then svelte2tsx's.
            serde_json::json!({
                "type": "ERROR", "filename": "src/Unclosed.svelte",
                "start": unclosed_at, "end": unclosed_to,
                "message": unclosed_message, "code": "element_unclosed",
                "codeDescription": {
                    "href": "https://svelte.dev/docs/svelte/compiler-errors#element_unclosed"
                },
                "source": "svelte",
            }),
            serde_json::json!({
                "type": "ERROR", "filename": "src/Unclosed.svelte",
                "start": unclosed_at, "end": unclosed_to,
                "message": unclosed_message, "code": -1, "source": "js",
            }),
        ],
        "a component svelte2tsx can parse stays dropped, as our parser had it"
    );
    // Ok, Unclosed and StrayClose; Parses stays out.
    assert!(
        completed.starts_with("COMPLETED 3 FILES 3 ERRORS"),
        "{completed}"
    );
}

#[test]
fn disable_enhance_keeps_the_tsgo_drop() {
    let dir = workspace(&[("src/Ok.svelte", OK), ("src/Unclosed.svelte", UNCLOSED)]);
    let (diagnostics, completed) = check(dir.path(), &["--disable-enhance"]);
    assert!(diagnostics.is_empty(), "{diagnostics:?}");
    assert!(
        completed.starts_with("COMPLETED 1 FILES 0 ERRORS"),
        "{completed}"
    );
}

#[test]
fn compile_option_error_comes_from_the_svelte_config_at_the_file_start() {
    let dir = workspace(&[
        ("src/Unclosed.svelte", UNCLOSED),
        (
            "svelte.config.js",
            "export default { compilerOptions: { dev: 'invalid' } };\n",
        ),
        ("package.json", r#"{ "type": "module" }"#),
    ]);
    let (diagnostics, _) = check(dir.path(), &[]);
    let found: Vec<_> = diagnostics
        .iter()
        .map(|d| (d["code"].clone(), d["start"].clone(), d["source"].clone()))
        .collect();
    assert_eq!(
        found,
        vec![
            (
                serde_json::json!("options_invalid_value"),
                serde_json::json!({ "line": 0, "character": 0 }),
                serde_json::json!("svelte"),
            ),
            (
                serde_json::json!(-1),
                serde_json::json!({ "line": 3, "character": 0 }),
                serde_json::json!("js"),
            ),
        ]
    );
}
