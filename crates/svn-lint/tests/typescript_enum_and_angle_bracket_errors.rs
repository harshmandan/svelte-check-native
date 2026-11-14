//! Errors acorn-typescript raises reading a TypeScript script, where
//! they come before the compiler's refusal of the construct they sit in
//! or before a later syntax error.
//!
//! - The compiler refuses an enum (`typescript_invalid_feature`) when it
//!   analyses the tree. A bad number, string, regular expression or
//!   reserved word inside the enum stops the parser first. The name of an
//!   enum is a binding (`enum let {}`); a member's name is not.
//! - Before it reads `<T>value` as a type assertion, acorn-typescript
//!   tries `<T>(…) => …`, an arrow function with type parameters. A type
//!   parameter's name is a binding, so `<let>y` fails that attempt on a
//!   reserved word. When the assertion then fails as well, the arrow
//!   attempt's error is the one thrown.
//!
//! Each case is what svelte 5.57.2's `compile()` throws: code, first
//! line of the message, and the byte range it points at.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32, u32)>;

const CASES: &[(&str, Expected)] = &[
    (
        "<script lang=\"ts\">x = <let>y + ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 23, 23)),
    ),
    (
        "<script lang=\"ts\">x = <T>y + ;</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = <T>(y) + ;</script>",
        Some(("js_parse_error", "Unexpected token", 31, 31)),
    ),
    (
        "<script lang=\"ts\">x = <T extends>y;</script>",
        Some(("js_parse_error", "Unexpected token", 32, 32)),
    ),
    (
        "<script lang=\"ts\">x = <yield>y + ;</script>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 23, 23)),
    ),
    (
        "<script lang=\"ts\">x = <T>y + 08;</script>",
        Some(("js_parse_error", "Invalid number", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = <let>y + 08;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 23, 23)),
    ),
    (
        "<script lang=\"ts\">x = <a.b>y + ;</script>",
        Some(("js_parse_error", "Unexpected token", 24, 24)),
    ),
    (
        "<script lang=\"ts\">x = <T>y;\nz = ;</script>",
        Some(("js_parse_error", "Unexpected token", 32, 32)),
    ),
    ("<script lang=\"ts\">f(<let>y, );</script>", None),
    (
        "<script lang=\"ts\">x = <string>y + ;</script>",
        Some(("js_parse_error", "Unexpected token", 34, 34)),
    ),
    (
        "<script lang=\"ts\">x = <static>(a) + ;</script>",
        Some(("js_parse_error", "The keyword 'static' is reserved", 23, 23)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 08 }</script>",
        Some(("js_parse_error", "Invalid number", 31, 31)),
    ),
    (
        "<script lang=\"ts\">enum E { A = \"\\08\" }</script>",
        Some(("js_parse_error", "Octal literal in strict mode", 32, 32)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 08, B = }</script>",
        Some(("js_parse_error", "Invalid number", 31, 31)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 1, B = 08 }</script>",
        Some(("js_parse_error", "Invalid number", 38, 38)),
    ),
    (
        "<script lang=\"ts\">enum E { let = 1 }</script>",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            18,
            36,
        )),
    ),
    (
        "<script lang=\"ts\">enum E { A = yield }</script>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 31, 31)),
    ),
    (
        "<script lang=\"ts\">enum E { A = /a)/ }</script>",
        Some((
            "js_parse_error",
            "Invalid regular expression: /a)/: Unmatched ')'",
            32,
            32,
        )),
    ),
    (
        "<script lang=\"ts\">enum let { A }</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 23, 23)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 1 }\nlet = 1;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 35, 35)),
    ),
    (
        "<script lang=\"ts\">let = 1;\nenum E { A }</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 18, 18)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 1 }\nx = 08;</script>",
        Some(("js_parse_error", "Invalid number", 39, 39)),
    ),
    (
        "<script lang=\"ts\">namespace N { export const a = 08; }</script>",
        Some(("js_parse_error", "Invalid number", 49, 49)),
    ),
    (
        "<script lang=\"ts\">namespace N { let = 1 }</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 32, 32)),
    ),
    (
        "<script lang=\"ts\">declare let x: number; x = 08;</script>",
        Some(("js_parse_error", "Invalid number", 45, 45)),
    ),
    (
        "<script lang=\"ts\">class A { constructor(private a = 08) {} }</script>",
        Some(("js_parse_error", "Invalid number", 52, 52)),
    ),
    (
        "<script lang=\"ts\">class A { constructor(private a) {} }\nx = 08;</script>",
        Some(("js_parse_error", "Invalid number", 60, 60)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 1__0 }</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            33,
            33,
        )),
    ),
    (
        "<script lang=\"ts\">const enum E { A = 08 }</script>",
        Some(("js_parse_error", "Invalid number", 37, 37)),
    ),
    (
        "<script lang=\"ts\">declare enum E { A = 08 }</script>",
        Some(("js_parse_error", "Invalid number", 39, 39)),
    ),
    (
        "<script lang=\"ts\">enum E { A = 1, A = 2 }</script>",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            18,
            41,
        )),
    ),
    (
        "<script lang=\"ts\">enum E { A }\nenum E { B = 08 }</script>",
        Some(("js_parse_error", "Invalid number", 44, 44)),
    ),
    (
        "<script lang=\"ts\">x = <T,>(a) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 26, 26)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a: T) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 32, 32)),
    ),
    (
        "<script lang=\"ts\">x = <T>() => ;</script>",
        Some(("js_parse_error", "Unexpected token", 25, 25)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a, b) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 32, 32)),
    ),
    (
        "<script lang=\"ts\">x = <T extends U>(a) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 35, 35)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a): T => ;</script>",
        Some(("js_parse_error", "Unexpected token", 28, 28)),
    ),
    (
        "<script lang=\"ts\">x = <T[]>y + ;</script>",
        Some(("js_parse_error", "Unexpected token", 24, 24)),
    ),
    (
        "<script lang=\"ts\">x = <{a: 1}>y + ;</script>",
        Some(("js_parse_error", "Unexpected token", 23, 23)),
    ),
    (
        "<script lang=\"ts\">x = 1 + <let>y + ;</script>",
        Some(("js_parse_error", "Unexpected token", 35, 35)),
    ),
    (
        "<script lang=\"ts\">f(<let>y + );</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 21, 21)),
    ),
    (
        "<script lang=\"ts\">x = <A>(<let>y + );</script>",
        Some(("js_parse_error", "Unexpected token", 25, 25)),
    ),
    (
        "<script lang=\"ts\">x = <let>(<B>y + );</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 23, 23)),
    ),
    (
        "<script lang=\"ts\">function f<let>() {}</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a) => { ( };</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a) => a + ;</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = <const T>(a) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 31, 31)),
    ),
    (
        "<script lang=\"ts\">x = <T>(...a) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 25, 25)),
    ),
    (
        "<script lang=\"ts\">x = <T>({a}) => ;</script>",
        Some(("js_parse_error", "Unexpected token", 31, 31)),
    ),
    (
        "<script lang=\"ts\">x = <string>y + 08;</script>",
        Some(("js_parse_error", "Invalid number", 34, 34)),
    ),
    (
        "<script lang=\"ts\">x = c ? <T>(a) => ; : 1</script>",
        Some(("js_parse_error", "Unexpected token", 33, 33)),
    ),
    (
        "<script lang=\"ts\">x = [<T>(a) => ;]</script>",
        Some(("js_parse_error", "Unexpected token", 30, 30)),
    ),
    (
        "<script lang=\"ts\">y = <T>(a) => { return ; }; z = ;</script>",
        Some(("js_parse_error", "Unexpected token", 50, 50)),
    ),
    (
        "<script lang=\"ts\">class A<let> {}</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 26, 26)),
    ),
    (
        "<script lang=\"ts\">class A { m<let>() {} }</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 30, 30)),
    ),
    (
        "<script lang=\"ts\">interface I<let> {}</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 30, 30)),
    ),
    (
        "<script lang=\"ts\">type T<let> = 1;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 25, 25)),
    ),
    (
        "<script lang=\"ts\">x = <let>(a) => a;</script>",
        Some(("js_parse_error", "Unexpected token", 31, 31)),
    ),
    (
        "<script lang=\"ts\">x = function <static>() {};</script>",
        Some(("js_parse_error", "The keyword 'static' is reserved", 32, 32)),
    ),
    (
        "<script lang=\"ts\">function f<T, yield>() {}</script>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 32, 32)),
    ),
    (
        "<script lang=\"ts\">function f<const let>() {}</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 35, 35)),
    ),
    (
        "<script lang=\"ts\">function f<T extends let>() {}</script>",
        None,
    ),
    (
        "<script lang=\"ts\">function f<T = let>() {}</script>",
        None,
    ),
    (
        "<script lang=\"ts\">x = { m<let>() {} };</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 26, 26)),
    ),
    // A declared function: a reserved word passes for a name there.
    (
        "<script lang=\"ts\">declare function f<let>(): void;</script>",
        None,
    ),
    (
        "<script lang=\"ts\">function f<await>() {}</script>",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            29,
            29,
        )),
    ),
    (
        "<script lang=\"ts\">function f<enum>() {}</script>",
        Some(("js_parse_error", "The keyword 'enum' is reserved", 29, 29)),
    ),
    (
        "<script lang=\"ts\">function f<private>() {}</script>",
        Some((
            "js_parse_error",
            "The keyword 'private' is reserved",
            29,
            29,
        )),
    ),
    (
        "<script lang=\"ts\">x = <T>(a) => { return 08 };</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a) => a;\ny = 08;</script>",
        Some(("js_parse_error", "Invalid number", 39, 39)),
    ),
    (
        "<script lang=\"ts\">x = <T, U>(a) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 28, 28)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a) => (b) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = (<T>(a) => { let = 1 });</script>",
        Some(("js_parse_error", "Unexpected token", 30, 30)),
    ),
    (
        "<script lang=\"ts\">const f = <T,>(a: T) => { yield = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 32, 32)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a?: T) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 33, 33)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a = 1) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 33, 33)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a: T = 1) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 36, 36)),
    ),
    (
        "<script lang=\"ts\">x = <T>({a}: P) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 34, 34)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a, ...b) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 25, 25)),
    ),
    (
        "<script lang=\"ts\">x = <T>(a,) => { let = 1 };</script>",
        Some(("js_parse_error", "Unexpected token", 25, 25)),
    ),
    (
        "<script lang=\"ts\">x = <any>y.z( ;</script>",
        Some(("js_parse_error", "Unexpected token", 27, 27)),
    ),
    (
        "<script lang=\"ts\">x = <any>(y, ;</script>",
        Some(("js_parse_error", "Unexpected token", 27, 27)),
    ),
    (
        "<script lang=\"ts\">x = <any>y;\nz = <let>y;</script>",
        None,
    ),
    (
        "<script lang=\"ts\">x = <T>y ? 08 : 1;</script>",
        Some(("js_parse_error", "Invalid number", 29, 29)),
    ),
    (
        "<script lang=\"ts\">x = (a, <let>y + );</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 27, 27)),
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
        let warnings = svn_lint::lint_file_with_options(
            src,
            Path::new("T.svelte"),
            options,
            CompatFeatures::MODERN,
        );
        let got = warnings.iter().find(|w| w.is_error).map(|w| {
            (
                w.code.as_str(),
                w.message.lines().next().unwrap_or_default(),
                w.range.start,
                w.range.end,
            )
        });
        if got != expected {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
