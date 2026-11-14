//! Inputs acorn-typescript breaks on, and what the compiler makes of
//! that.
//!
//! In a few places acorn-typescript reads a field of a node it does not
//! have: the member a decorator belongs to when a lone `;` follows it,
//! a parameter of a signature where two commas leave a hole, the name
//! of an exported declaration that is no declaration. The JavaScript
//! engine then throws its own error, with its own message and no place
//! in the text. After `a?.<T>` without a call it raises "Unexpected
//! token" with a line and column where an offset belongs, which leaves
//! the error without a usable place as well.
//!
//! The compiler reports all of these as `js_parse_error` with that
//! message and no position. A check run shows such an error at the
//! start of the file, which is what the ranges here say.
//!
//! A script is read where it stands in the component. The last cases
//! pair a script error with a template error: the one the parser
//! reaches first is thrown, wherever it then points.
//!
//! Each case is what svelte 5.57.2's `compile()` throws: code, first
//! line of the message, and the byte range it points at (`0, 0` where
//! it points at none). An empty code is a JavaScript exception that
//! `compile()` let through: inside an `{#each}` head the compiler asks
//! the error for its position, and breaks in turn when there is none.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32, u32)>;

const CASES: &[(&str, Expected)] = &[
    (
        "<script lang=\"ts\">class A { @dec ; }</script>",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">class A { @dec ; m() {} }</script>",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">class A { @a @b ; }</script>",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">class A { ; @dec m() {} }</script>",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            30,
            34,
        )),
    ),
    (
        "<script lang=\"ts\">class A { @dec }</script>",
        Some((
            "js_parse_error",
            "Decorators must be attached to a class element.",
            34,
            34,
        )),
    ),
    (
        "<script lang=\"ts\">x = class { @dec ; };</script>",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">let a = 1;\nclass A {\n  @dec ;\n}</script>",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">type F = (a,,b) => void;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of null (reading 'type')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">interface I { m(a,,b): void }</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of null (reading 'type')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">interface I { new (,a): I }</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of null (reading 'type')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">type F = new (a,,) => void;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of null (reading 'type')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">declare function f(a,,b): void;</script>",
        Some(("js_parse_error", "Unexpected token", 39, 39)),
    ),
    (
        "<script lang=\"ts\">let x: [a,,b];</script>",
        Some(("js_parse_error", "Unexpected token", 28, 28)),
    ),
    (
        "<script lang=\"ts\">type F = (,) => void;</script>",
        Some(("js_parse_error", "Unexpected token", 28, 28)),
    ),
    ("<script lang=\"ts\">type F = (a,) => void;</script>", None),
    (
        "<script lang=\"ts\">export type;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">export namespace;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">export type\nfoo</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">export abstract\nclass A {}</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">export type: x</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">export module;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">export declare;</script>",
        Some((
            "js_parse_error",
            "'export declare' must be followed by an ambient declaration.",
            32,
            32,
        )),
    ),
    (
        "<script lang=\"ts\">export interface\nI {}</script>",
        Some((
            "js_parse_error",
            "The keyword 'interface' is reserved",
            25,
            25,
        )),
    ),
    (
        "<script lang=\"ts\">export enum;</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    ("<script lang=\"ts\">export type T = 1;</script>", None),
    (
        "<script lang=\"ts\">export abstract class A {}</script>",
        None,
    ),
    (
        "<script lang=\"ts\">let a = 08;\nexport type;</script>",
        Some(("js_parse_error", "Invalid number", 26, 26)),
    ),
    (
        "<script lang=\"ts\">export type;\nlet a = 08;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">x = a?.<T>b;</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    ("<script lang=\"ts\">x = a?.<T>(b);</script>", None),
    (
        "<script lang=\"ts\">x = a?.<T>;</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\">x = <U>(y) => a?.<T>b;</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = a?.<T>b + 08;</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\">x = 08 + a?.<T>b;</script>",
        Some(("js_parse_error", "Invalid number", 22, 22)),
    ),
    (
        "<script lang=\"ts\">x = f(a?.<T>b);</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\">x = c ? a?.<T>b : 1;</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\">x = async <U>(y) => a?.<T>b;</script>",
        Some(("js_parse_error", "Unexpected token", 35, 35)),
    ),
    (
        "<script lang=\"ts\">x = (y): a?.<T>b => 1;</script>",
        Some(("js_parse_error", "Unexpected token", 25, 25)),
    ),
    (
        "<script lang=\"ts\"></script>\n{a?.<T>b}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{(class { @dec ; })}",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#if a?.<T>b}x{/if}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each a?.<T>b as x}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as x (a?.<T>b)}x{/each}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{@const x = a?.<T>b}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#if true}{@const x = a?.<T>b}{/if}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n<div a={a?.<T>b}></div>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n<div {...a?.<T>b}></div>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{@html a?.<T>b}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#await a?.<T>b}x{/await}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#key a?.<T>b}x{/key}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#snippet s(a = b?.<T>c)}x{/snippet}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{@render s(a?.<T>b)}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as { a = class { @dec ; } }}x{/each}",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each (class { @dec ; }) as x}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n<div onclick={() => a?.<T>b}></div>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{a}\n{a?.<T>b}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{08}{a?.<T>b}",
        Some(("js_parse_error", "Invalid number", 29, 29)),
    ),
    (
        "<script lang=\"ts\"></script>\n{a?.<T>b}{08}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{(x as (a,,b) => void)}",
        Some((
            "js_parse_error",
            "Cannot read properties of null (reading 'type')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#await p then { a = class { @dec ; } }}x{/await}",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n<div class:a={a?.<T>b}></div>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n<input bind:value={a?.<T>b} />",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#snippet s(a: (x,,y) => void)}x{/snippet}",
        Some((
            "js_parse_error",
            "Cannot read properties of null (reading 'type')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{@debug a?.<T>b}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\" module>export type;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\" module>let a = 08;</script><script lang=\"ts\">export type;</script>",
        Some(("js_parse_error", "Invalid number", 33, 33)),
    ),
    (
        "<script lang=\"ts\" module>export type;</script><script lang=\"ts\">let a = 08;</script>",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "<script>x = a?.<T>b;</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script lang=\"ts\">export type;</script>{08}",
        Some((
            "js_parse_error",
            "Cannot read properties of undefined (reading 'start')",
            0,
            0,
        )),
    ),
    (
        "{08}<script lang=\"ts\">export type;</script>",
        Some(("js_parse_error", "Invalid number", 1, 1)),
    ),
    (
        "<script lang=\"ts\">let a = 08;</script>{a?.<T>b}",
        Some(("js_parse_error", "Invalid number", 26, 26)),
    ),
    (
        "{a?.<T>b}<script lang=\"ts\">let a = 08;</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<div>{a?.<T>b}</div><script lang=\"ts\">export type;</script>",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as (a,,b) => void}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as { m(a,,b): void }}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as x, a?.<T>b}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each a?.<T>b}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each f(a as b?.<T>c) as x}x{/each}",
        Some(("js_parse_error", "Unexpected token", 39, 39)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as [a = b?.<T>c]}x{/each}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each a?.<T>(b) as x}x{/each}",
        None,
    ),
    (
        "<script lang=\"ts\"></script>\n{#each 08 as (a,,b) => void}x{/each}",
        Some(("js_parse_error", "Invalid number", 35, 35)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as x}{a?.<T>b}{/each}",
        Some(("js_parse_error", "Unexpected token (1:NaN)", 0, 0)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as x (class { @dec ; })}x{/each}",
        Some((
            "js_parse_error",
            "Cannot set properties of null (setting 'decorators')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each xs as\n  { m(a,,b): void }}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each as as (a,,b) => void}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each (a as (a,,b) => void) as x}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each [a?.<T>b] as x}x{/each}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{#if a}{#each a?.<T>b as x}x{/each}{/if}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\"></script>\n{08}{#each a?.<T>b as x}x{/each}",
        Some(("js_parse_error", "Invalid number", 29, 29)),
    ),
    (
        "<script lang=\"ts\"></script>\n{#each a?.<T>b as x}x{/each}{08}",
        Some((
            "",
            "Cannot read properties of undefined (reading '0')",
            0,
            0,
        )),
    ),
    (
        "<script lang=\"ts\">let a = ;</script>{08}",
        Some(("js_parse_error", "Unexpected token", 26, 26)),
    ),
    (
        "{08}<script lang=\"ts\">let a = ;</script>",
        Some(("js_parse_error", "Invalid number", 1, 1)),
    ),
    (
        "<script>let a = ;</script>{08}",
        Some(("js_parse_error", "Unexpected token", 16, 16)),
    ),
    (
        "{08}<script>let a = ;</script>",
        Some(("js_parse_error", "Invalid number", 1, 1)),
    ),
    (
        "<script lang=\"ts\">let a = ;</script><div {08}></div>",
        Some(("js_parse_error", "Unexpected token", 26, 26)),
    ),
    (
        "<p>{08}</p>\n<script lang=\"ts\">let a = ;</script>",
        Some(("js_parse_error", "Invalid number", 4, 4)),
    ),
];

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(src, expected) in CASES {
        let options = svn_lint::LintOptions {
            preprocess_configured: true,
            ..svn_lint::LintOptions::default()
        };
        let (warnings, exception) = svn_lint::lint_file_or_exception(
            src,
            Path::new("T.svelte"),
            options,
            CompatFeatures::MODERN,
        );
        let got = match &exception {
            Some(message) => Some(("", message.as_str(), 0, 0)),
            None => warnings.iter().find(|w| w.is_error).map(|w| {
                (
                    w.code.as_str(),
                    w.message.lines().next().unwrap_or_default(),
                    w.range.start,
                    w.range.end,
                )
            }),
        };
        if got != expected {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
