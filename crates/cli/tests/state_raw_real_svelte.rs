//! Black-box coverage for `$state.raw` when a real Svelte package is
//! resolvable. This exercises the branch that strips the native
//! `$state` ambients and leaves Svelte's declarations authoritative.

#![allow(clippy::expect_used)]

use std::fs;
use std::process::Command;

#[test]
fn installed_svelte_owns_state_raw_overloads() {
    let workspace = tempfile::tempdir().expect("temp workspace");
    let root = workspace.path();
    let svelte = root.join("node_modules/svelte");
    fs::create_dir_all(&svelte).expect("create mock Svelte package");

    fs::write(
        svelte.join("package.json"),
        r#"{
            "name": "svelte",
            "version": "5.56.5",
            "types": "./index.d.ts",
            "exports": {
                ".": {
                    "types": "./index.d.ts"
                },
                "./elements": {
                    "types": "./elements.d.ts"
                }
            }
        }"#,
    )
    .expect("write Svelte package manifest");
    fs::write(
        svelte.join("index.d.ts"),
        r#"export type Component<Props = any, Exports = any, Bindings = any> =
    (internals: unknown, props: Props) => Exports;
export declare class SvelteComponent<
    Props = Record<string, any>,
    Events = Record<string, any>,
    Slots = Record<string, any>
> {}
export type Snippet<Parameters extends unknown[] = []> =
    (...args: Parameters) => unknown;

declare global {
    function $state<T>(initial: T): T;
    function $state<T>(): T | undefined;
    namespace $state {
        function raw<T>(initial: T): T;
        function raw<T>(): T | undefined;
    }
}
"#,
    )
    .expect("write Svelte state declarations");
    fs::write(
        svelte.join("elements.d.ts"),
        r#"export interface HTMLAttributes<T> {
    [name: string]: any;
}

export interface SvelteHTMLElements {
    [name: string]: HTMLAttributes<any>;
}
"#,
    )
    .expect("write Svelte element declarations");

    fs::write(
        root.join("state.svelte.ts"),
        r#"import type {} from 'svelte';

export const selected = $state.raw<ReadonlySet<string>>(new Set());
export const invalid = $state.raw<number>('not a number');
"#,
    )
    .expect("write runes module");
    let tsconfig = root.join("tsconfig.json");
    fs::write(
        &tsconfig,
        r#"{
            "compilerOptions": {
                "target": "ES2022",
                "module": "ESNext",
                "moduleResolution": "bundler",
                "strict": true
            },
            "include": ["**/*"]
        }"#,
    )
    .expect("write tsconfig");

    let bin = env!("CARGO_BIN_EXE_svelte-check-native");
    let crate_dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    let repo_root = crate_dir
        .parent()
        .and_then(std::path::Path::parent)
        .expect("CLI crate must live under the repository root");
    let tsgo = svn_typecheck::discover(repo_root)
        .expect("could not locate native TypeScript; install dependencies or set TSGO_BIN");

    let output = Command::new(bin)
        .args([
            "--workspace",
            root.to_str().expect("UTF-8 workspace path"),
            "--tsconfig",
            tsconfig.to_str().expect("UTF-8 tsconfig path"),
            "--output",
            "machine-verbose",
        ])
        .env("TSGO_BIN", tsgo.path)
        .output()
        .expect("run svelte-check-native");

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        output.status.code(),
        Some(1),
        "the intentional type error should set exit code 1; stdout:\n{stdout}"
    );
    let error_count = stdout.matches(r#""type":"ERROR""#).count();
    assert_eq!(
        error_count, 1,
        "expected only the intentional mismatch; stdout:\n{stdout}"
    );
    assert!(
        stdout.contains(r#""code":2345"#),
        "expected direct assignability error; stdout:\n{stdout}"
    );
    assert!(
        !stdout.contains(r#""code":2769"#),
        "duplicate/legacy overloads must not produce TS2769; stdout:\n{stdout}"
    );
}
