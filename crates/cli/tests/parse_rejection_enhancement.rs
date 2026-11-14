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

/// The default engine diagnoses the files of the TypeScript program, which
/// follows imports past the tsconfig's `include`: a rejected component
/// outside it is reported when a file of the program imports it, and not
/// otherwise.
#[test]
fn rejected_component_outside_the_tsconfig_counts_when_imported() {
    let dir = workspace(&[
        (
            "src/App.svelte",
            "<script lang=\"ts\">\n  import Imported from '../lib/Imported.svelte';\n</script>\n<Imported />\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        ("lib/Imported.svelte", UNCLOSED),
        ("lib/Lonely.svelte", UNCLOSED),
    ]);
    let (diagnostics, completed) = check(dir.path(), &[]);
    let found: Vec<_> = diagnostics
        .iter()
        .map(|d| (d["filename"].clone(), d["code"].clone()))
        .collect();
    assert_eq!(
        found,
        vec![
            (
                serde_json::json!("lib/Imported.svelte"),
                serde_json::json!("element_unclosed"),
            ),
            (
                serde_json::json!("lib/Imported.svelte"),
                serde_json::json!(-1)
            ),
            // Its script has no `import` / `export`: not a module.
            (serde_json::json!("src/App.svelte"), serde_json::json!(2306)),
        ]
    );
    // App and Imported; Lonely stays out.
    assert!(
        completed.starts_with("COMPLETED 2 FILES 3 ERRORS"),
        "{completed}"
    );
    // A second run replays tsgo's saved result, program included.
    assert_eq!(check(dir.path(), &[]), (diagnostics, completed));
}

/// The default engine gives a rejected component's importers its script
/// alone (the instance script, else the module script). Without
/// `import` / `export` that is no module, which TypeScript reports at the
/// specifier; with them it is a module without the component's default
/// export or its other names.
#[test]
fn importers_of_a_rejected_component_see_its_script() {
    let dir = workspace(&[
        (
            "src/App.svelte",
            "<script lang=\"ts\">\n  import NoModule from './NoModule.svelte';\n  import WithImport, { named, missing } from './WithImport.svelte';\n  import NoScript from './NoScript.svelte';\n  import * as ns from './NoModule.svelte';\n  import { default as X } from './WithImport.svelte';\n  const lazy = import('./NoModule.svelte');\n  void [NoModule, WithImport, named, missing, NoScript, ns, X, lazy];\n</script>\n",
        ),
        (
            "src/more.ts",
            "export { default as A } from './NoModule.svelte';\nexport { missing as C, named as D } from './WithImport.svelte';\nexport * from './WithImport.svelte';\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        ("src/NoModule.svelte", UNCLOSED),
        (
            "src/WithImport.svelte",
            "<script>\n  import { tick } from './tick';\n  export const named = tick;\n</script>\n<div>\n",
        ),
        ("src/tick.ts", "export const tick = 1;\n"),
        ("src/NoScript.svelte", "<div>\n"),
    ]);
    let (diagnostics, _) = check(dir.path(), &[]);
    let file = |name: &str| named_file(dir.path(), name);
    let mut found: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d["code"].as_i64(), Some(2306 | 1192 | 2305)))
        .map(|d| {
            (
                d["filename"].as_str().unwrap().to_string(),
                d["start"]["line"].as_i64().unwrap(),
                d["start"]["character"].as_i64().unwrap(),
                d["code"].as_i64().unwrap(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    found.sort();
    let not_module = format!("File '{}' is not a module.", file("NoModule.svelte"));
    let no_default = format!(
        "Module '\"{}\"' has no default export.",
        file("WithImport.svelte")
    );
    let mut want = vec![
        (
            "src/App.svelte".to_string(),
            1,
            23,
            2306,
            not_module.clone(),
        ),
        ("src/App.svelte".to_string(), 2, 9, 1192, no_default),
        (
            "src/App.svelte".to_string(),
            2,
            30,
            2305,
            "Module '\"./WithImport.svelte\"' has no exported member 'missing'.".to_string(),
        ),
        (
            "src/App.svelte".to_string(),
            3,
            23,
            2306,
            format!("File '{}' is not a module.", file("NoScript.svelte")),
        ),
        (
            "src/App.svelte".to_string(),
            4,
            22,
            2306,
            not_module.clone(),
        ),
        (
            "src/App.svelte".to_string(),
            5,
            11,
            2305,
            "Module '\"./WithImport.svelte\"' has no exported member 'default'.".to_string(),
        ),
        (
            "src/App.svelte".to_string(),
            6,
            22,
            2306,
            not_module.clone(),
        ),
        ("src/more.ts".to_string(), 0, 29, 2306, not_module),
        (
            "src/more.ts".to_string(),
            1,
            9,
            2305,
            "Module '\"./WithImport.svelte\"' has no exported member 'missing'.".to_string(),
        ),
    ];
    want.sort();
    assert_eq!(found, want);
}

/// A file under `src`, as TypeScript names it in a message: resolved,
/// with forward slashes and no Windows `\\?\` prefix.
fn named_file(root: &Path, name: &str) -> String {
    dunce::canonicalize(root)
        .unwrap()
        .join("src")
        .join(name)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Places TypeScript says nothing about a rejected component: where it
/// is imported only to run it, and in a declaration file the project
/// does not check. A script that reads `import.meta` is a module, so
/// what is missing from it is a default export, not module-ness.
#[test]
fn importers_typescript_does_not_question() {
    let dir = workspace(&[
        (
            "src/App.svelte",
            "<script lang=\"ts\">\n  import './NoModule.svelte';\n  import Meta from './Meta.svelte';\n  void Meta;\n</script>\n",
        ),
        (
            "src/types.d.ts",
            "import type Bad from './NoModule.svelte';\nexport type T = Bad;\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        ("src/NoModule.svelte", UNCLOSED),
        (
            "src/Meta.svelte",
            "<script>\n  const u = import.meta.url;\n</script>\n<div>{u}\n",
        ),
    ]);
    let (diagnostics, _) = check(dir.path(), &[]);
    let found: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d["code"].as_i64(), Some(2306 | 1192 | 2305)))
        .map(|d| {
            (
                d["filename"].as_str().unwrap().to_string(),
                d["start"]["line"].as_i64().unwrap(),
                d["start"]["character"].as_i64().unwrap(),
                d["code"].as_i64().unwrap(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    let want = vec![(
        "src/App.svelte".to_string(),
        2,
        9,
        1192,
        format!(
            "Module '\"{}\"' has no default export.",
            named_file(dir.path(), "Meta.svelte")
        ),
    )];
    assert_eq!(found, want);
}

/// A `// @ts-ignore` or `// @ts-expect-error` above an import takes the
/// error TypeScript would give there. The expectation is met, so it is
/// not reported as unused either.
#[test]
fn comment_directives_above_an_import_take_its_error() {
    let dir = workspace(&[
        (
            "src/App.svelte",
            "<script lang=\"ts\">\n  // @ts-ignore\n  import A from './NoModule.svelte';\n  // @ts-expect-error\n  import B from './NoModule.svelte';\n\n  // @ts-expect-error\n  // a note\n  import C from './NoModule.svelte';\n  import D from './NoModule.svelte';\n  void [A, B, C, D];\n</script>\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        ("src/NoModule.svelte", UNCLOSED),
    ]);
    let (diagnostics, _) = check(dir.path(), &[]);
    let found: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d["code"].as_i64(), Some(2306 | 1192 | 2305 | 2578)))
        .map(|d| {
            (
                d["start"]["line"].as_i64().unwrap(),
                d["start"]["character"].as_i64().unwrap(),
                d["code"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(found, [(9, 16, 2306)]);
}

/// TypeScript reads a directive in a block comment too: on the comment's
/// last line, after any leading `/` and `*`. A line comment is one only
/// with two or three slashes. Each expectation is TypeScript 6.0.3's for
/// the same lines in a plain module.
#[test]
fn block_comment_directives_above_an_import_take_its_error() {
    let dir = workspace(&[
        (
            "src/App.svelte",
            "<script lang=\"ts\">\n  /* @ts-ignore */\n  import A from './NoModule.svelte';\n  /*\n   * @ts-expect-error */\n  import B from './NoModule.svelte';\n  //// @ts-ignore\n  import C from './NoModule.svelte';\n  const s = \"//\"; /* @ts-ignore */\n  import D from './NoModule.svelte';\n  void [A, B, C, D, s];\n</script>\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        ("src/NoModule.svelte", UNCLOSED),
    ]);
    let (diagnostics, _) = check(dir.path(), &[]);
    let found: Vec<_> = diagnostics
        .iter()
        .filter(|d| matches!(d["code"].as_i64(), Some(2306 | 1192 | 2305 | 2578)))
        .map(|d| {
            (
                d["start"]["line"].as_i64().unwrap(),
                d["start"]["character"].as_i64().unwrap(),
                d["code"].as_i64().unwrap(),
            )
        })
        .collect();
    assert_eq!(found, [(7, 16, 2306)]);
}

/// What tsgo says of a name imported from a rejected component is set
/// aside only where it judged the name against svelte's `*.svelte`
/// declaration. What does not depend on the module, such as an unused
/// import under `noUnusedLocals`, the default engine reports as well.
#[test]
fn an_unused_import_of_a_rejected_component_is_still_reported() {
    let dir = workspace(&[
        (
            "src/main.ts",
            "import Z, { named } from './WithImport.svelte';\nexport const n = named;\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        (
            "src/WithImport.svelte",
            "<script>\n  export const named = 1;\n</script>\n<div>\n",
        ),
    ]);
    fs::write(
        dir.path().join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ESNext",
             "moduleResolution": "bundler", "strict": true, "skipLibCheck": true,
             "noUnusedLocals": true },
             "include": ["src/**/*"] }"#,
    )
    .unwrap();
    let (diagnostics, _) = check(dir.path(), &[]);
    let mut found: Vec<_> = diagnostics
        .iter()
        .filter(|d| d["filename"] == "src/main.ts")
        .map(|d| {
            (
                d["start"]["line"].as_i64().unwrap(),
                d["start"]["character"].as_i64().unwrap(),
                d["code"].as_i64().unwrap(),
            )
        })
        .collect();
    found.sort();
    assert_eq!(found, [(0, 7, 1192), (0, 7, 6133)]);
}

/// What TypeScript says of a name a module does not export depends on
/// what the module does hold: an export spelled nearly alike, a default
/// export, a declaration it keeps to itself or exports under another
/// name. Each expectation is TypeScript 6.0.3's for the same imports of
/// plain modules with these scripts.
#[test]
fn a_missing_member_gets_the_error_typescript_chooses() {
    let dir = workspace(&[
        (
            "src/main.ts",
            "import { tick, fooBr, local, nothing, renamed, FOOBAR, foobarx, fo } from \"./Bad.svelte\";\nimport { tick as t2, zzz, local as l2, defalt } from \"./Def.svelte\";\nexport { fooBr as q, local as r } from \"./Bad.svelte\";\nvoid [tick, fooBr, local, nothing, renamed, FOOBAR, foobarx, fo, t2, zzz, l2, defalt];\n",
        ),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
        (
            "src/Bad.svelte",
            "<script lang=\"ts\">\n  import { tick } from \"./tick\";\n  export const fooBar = tick;\n  const local = 1;\n  const renamed = 2;\n  export { renamed as other };\n  void local;\n</script>\n<div>\n",
        ),
        (
            "src/Def.svelte",
            "<script lang=\"ts\">\n  import { tick } from \"./tick\";\n  const local = tick;\n  export default local;\n</script>\n<div>\n",
        ),
        ("src/tick.ts", "export const tick = 1;\n"),
    ]);
    let (diagnostics, _) = check(dir.path(), &[]);
    let mut found: Vec<_> = diagnostics
        .iter()
        .filter(|d| d["filename"] == "src/main.ts")
        .filter(|d| matches!(d["code"].as_i64(), Some(2305 | 2459 | 2460 | 2614 | 2724)))
        .map(|d| {
            (
                d["start"]["line"].as_i64().unwrap(),
                d["start"]["character"].as_i64().unwrap(),
                d["code"].as_i64().unwrap(),
                d["message"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    found.sort();
    let bad = "\"./Bad.svelte\"";
    let def = "\"./Def.svelte\"";
    let local_only =
        |name: &str| format!("Module '{bad}' declares '{name}' locally, but it is not exported.");
    let did_you_mean = |module: &str, name: &str, other: &str| {
        format!("'{module}' has no exported member named '{name}'. Did you mean '{other}'?")
    };
    let no_member = |name: &str| format!("Module '{bad}' has no exported member '{name}'.");
    let import_default = |name: &str| {
        format!(
            "Module '{def}' has no exported member '{name}'. Did you mean to use 'import {name} from {def}' instead?"
        )
    };
    let want = vec![
        (0, 9, 2459, local_only("tick")),
        (0, 15, 2724, did_you_mean(bad, "fooBr", "fooBar")),
        (0, 22, 2459, local_only("local")),
        (0, 29, 2305, no_member("nothing")),
        (
            0,
            38,
            2460,
            format!("Module '{bad}' declares 'renamed' locally, but it is exported as 'other'."),
        ),
        (0, 47, 2724, did_you_mean(bad, "FOOBAR", "fooBar")),
        (0, 55, 2724, did_you_mean(bad, "foobarx", "fooBar")),
        (0, 64, 2305, no_member("fo")),
        (1, 9, 2614, import_default("tick")),
        (1, 21, 2614, import_default("zzz")),
        (1, 26, 2614, import_default("local")),
        (1, 39, 2724, did_you_mean(def, "defalt", "local")),
        (2, 9, 2724, did_you_mean(bad, "fooBr", "fooBar")),
        (2, 21, 2459, local_only("local")),
    ];
    assert_eq!(found, want);
}

/// A rejected component is a file of the default engine's program (as
/// the script it falls back to), so a component it imports is in the
/// program too, outside the tsconfig or not, and one that imports in
/// turn.
#[test]
fn imports_are_followed_through_rejected_components() {
    let importing = |target: &str| {
        format!(
            "<script>\n  import Next from '{target}';\n  export const next = Next;\n</script>\n<div>\n"
        )
    };
    let dir = workspace(&[
        ("src/First.svelte", &importing("../lib/Second.svelte")),
        ("lib/Second.svelte", &importing("./Third.svelte")),
        ("lib/Third.svelte", UNCLOSED),
        ("lib/Lonely.svelte", UNCLOSED),
        (
            "src/ambient.d.ts",
            "declare module '*.svelte' {\n  const component: any;\n  export default component;\n}\n",
        ),
    ]);
    let (diagnostics, completed) = check(dir.path(), &[]);
    let mut files: Vec<String> = diagnostics
        .iter()
        .filter(|d| d["code"] == "element_unclosed")
        .map(|d| d["filename"].as_str().unwrap().to_string())
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["lib/Second.svelte", "lib/Third.svelte", "src/First.svelte"]
    );
    assert!(completed.starts_with("COMPLETED 3 FILES"), "{completed}");
}
