//! Errors acorn raises before it reaches a syntax error.
//!
//! acorn raises an early error where it meets one: a reserved word when
//! it reads the name, an octal literal or a misplaced numeric separator
//! when it reads the number, a name declared twice when it reads the
//! second declaration. A syntax error later in the same statement, or
//! later in the same tag, comes second. So `let = ;` is a reserved word
//! at `let`, and `{#each let as x, }` is one too, not a missing index
//! name.
//!
//! Also here, since they came up measuring the above:
//! - a function or class declaration as the body of a label or a loop,
//!   which strict code does not have;
//! - numeric literals as acorn's tokenizer reads them: a digit the radix
//!   has no use for ends the number, and each misuse of a separator has
//!   its own message.
//!
//! Each case is what svelte 5.57.2's `compile()` throws: code, first
//! line of the message, and the byte range it points at.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32, u32)>;

const CASES: &[(&str, Expected)] = &[
    (
        "<script>let = ; let a = /a)/;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 8, 8)),
    ),
    (
        "<script>let = ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 8, 8)),
    ),
    (
        "<script>yield = ;</script>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 8, 8)),
    ),
    (
        "<script>x = yield + ;</script>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 12, 12)),
    ),
    (
        "<script>static = ;</script>",
        Some(("js_parse_error", "The keyword 'static' is reserved", 8, 8)),
    ),
    (
        "<script>var package = ;</script>",
        Some((
            "js_parse_error",
            "The keyword 'package' is reserved",
            12,
            12,
        )),
    ),
    (
        "<script>a = implements + ;</script>",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved",
            12,
            12,
        )),
    ),
    (
        "<script>let x = 08 + ;</script>",
        Some(("js_parse_error", "Invalid number", 16, 16)),
    ),
    (
        "<script>let x = \"\\08\" + ;</script>",
        Some(("js_parse_error", "Octal literal in strict mode", 17, 17)),
    ),
    (
        "<script>delete x; (</script>",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            8,
            8,
        )),
    ),
    (
        "<script>delete x + ;</script>",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            8,
            8,
        )),
    ),
    (
        "<script>let a; let a = ;</script>",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            19,
            19,
        )),
    ),
    (
        "<script>let a, a = ;</script>",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            15,
            15,
        )),
    ),
    (
        "<script>arguments = ;</script>",
        Some((
            "js_parse_error",
            "Assigning to arguments in strict mode",
            8,
            8,
        )),
    ),
    (
        "<script>eval = ;</script>",
        Some(("js_parse_error", "Assigning to eval in strict mode", 8, 8)),
    ),
    (
        "<script>eval++ + ;</script>",
        Some(("js_parse_error", "Assigning to eval in strict mode", 8, 8)),
    ),
    (
        "<script>x = 1; arguments = ;</script>",
        Some((
            "js_parse_error",
            "Assigning to arguments in strict mode",
            15,
            15,
        )),
    ),
    (
        "<script>class A { constructor(){} constructor(){} foo( }</script>",
        Some((
            "js_parse_error",
            "Duplicate constructor in the same class",
            34,
            34,
        )),
    ),
    (
        "<script>class A { #a; #a; foo( }</script>",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            22,
            22,
        )),
    ),
    (
        "<script>class A { foo() { this.#b } bar( }</script>",
        Some(("js_parse_error", "Unexpected token", 41, 41)),
    ),
    (
        "<script>function f(a, a) { ( }</script>",
        Some(("js_parse_error", "Argument name clash", 22, 22)),
    ),
    (
        "<script>function f(a, a) {} (</script>",
        Some(("js_parse_error", "Argument name clash", 22, 22)),
    ),
    (
        "<script>var o = { __proto__: 1, __proto__: 2 }; (</script>",
        Some((
            "js_parse_error",
            "Redefinition of __proto__ property",
            32,
            32,
        )),
    ),
    (
        "<script>a: a: ; (</script>",
        Some(("js_parse_error", "Label 'a' is already declared", 11, 11)),
    ),
    (
        "<script>a: { a: ; } (</script>",
        Some(("js_parse_error", "Label 'a' is already declared", 13, 13)),
    ),
    (
        "<script>break; (</script>",
        Some(("js_parse_error", "Unsyntactic break", 8, 8)),
    ),
    (
        "<script>if (x) break; else (</script>",
        Some(("js_parse_error", "Unsyntactic break", 15, 15)),
    ),
    (
        "<script>continue; (</script>",
        Some(("js_parse_error", "Unsyntactic continue", 8, 8)),
    ),
    (
        "<script>function f() { new.target } new.target; (</script>",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block",
            36,
            36,
        )),
    ),
    (
        "<script>return; (</script>",
        Some(("js_parse_error", "'return' outside of function", 8, 8)),
    ),
    (
        "<script>super.x; (</script>",
        Some(("js_parse_error", "'super' keyword outside a method", 8, 8)),
    ),
    (
        "<script>with (a) {} (</script>",
        Some(("js_parse_error", "'with' in strict mode", 8, 8)),
    ),
    (
        "<script>function f() { \"use strict\"; with (a) {} } (</script>",
        Some(("js_parse_error", "'with' in strict mode", 37, 37)),
    ),
    (
        "<script>let let = 1; (</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 12, 12)),
    ),
    (
        "<script>const a; (</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>const a, b = ;</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>for (let let of x) ; (</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 17, 17)),
    ),
    (
        "<script>label: function f() {} (</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>if (a) function f() {} (</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>import a from \"a\"; import a from \"b\"; (</script>",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            34,
            34,
        )),
    ),
    (
        "<script>export { zz }; (</script>",
        Some(("js_parse_error", "Unexpected token", 24, 24)),
    ),
    (
        "<script>export default 1; export default 2; (</script>",
        Some(("js_parse_error", "Duplicate export 'default'", 33, 33)),
    ),
    (
        "<script>let a = 1; export { a, a }; (</script>",
        Some(("js_parse_error", "Duplicate export 'a'", 31, 31)),
    ),
    (
        "<script>async function f() { await = ; }</script>",
        Some(("js_parse_error", "Unexpected token", 35, 35)),
    ),
    (
        "<script>function* g() { yield = ; }</script>",
        Some(("js_parse_error", "Unexpected token", 30, 30)),
    ),
    (
        "<script>function* g() { var yield; ( }</script>",
        Some((
            "js_parse_error",
            "Cannot use 'yield' as identifier inside a generator",
            28,
            28,
        )),
    ),
    (
        "<script>var await; (</script>",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            12,
            12,
        )),
    ),
    (
        "<script>await: ; (</script>",
        Some(("js_parse_error", "Unexpected token", 13, 13)),
    ),
    (
        "<script>x = { a, enum } + ;</script>",
        Some(("js_parse_error", "The keyword 'enum' is reserved", 17, 17)),
    ),
    (
        "<script>x = { let } + ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 14, 14)),
    ),
    (
        "<script>x = { static } ; (</script>",
        Some(("js_parse_error", "The keyword 'static' is reserved", 14, 14)),
    ),
    (
        "<script>const { let } = ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 16, 16)),
    ),
    (
        "<script>f(let, ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 10, 10)),
    ),
    (
        "<script>[let, ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 9, 9)),
    ),
    (
        "<script>`${let}` + ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 11, 11)),
    ),
    (
        "<script>let.x + ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 8, 8)),
    ),
    (
        "<script>x.let + ;</script>",
        Some(("js_parse_error", "Unexpected token", 16, 16)),
    ),
    (
        "<script>x = 0b12 + ;</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>x = 1_ + ;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            13,
            13,
        )),
    ),
    (
        "<script>x = 1__0 + ;</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            14,
            14,
        )),
    ),
    (
        "<script>x = \"\\u{110000}\" + ;</script>",
        Some(("js_parse_error", "Code point out of bounds", 16, 16)),
    ),
    (
        "<script>x = `\\08` + ;</script>",
        Some((
            "js_parse_error",
            "Bad escape sequence in untagged template literal",
            13,
            13,
        )),
    ),
    (
        "<script>x = 0o8 + ;</script>",
        Some(("js_parse_error", "Expected number in radix 8", 14, 14)),
    ),
    (
        "<script>x = 1n.x.08 + ;</script>",
        Some(("js_parse_error", "Unexpected token", 16, 16)),
    ),
    (
        "<script>a ?? b || c; (</script>",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            15,
            15,
        )),
    ),
    (
        "<script>x = a ?? b || ;</script>",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            19,
            19,
        )),
    ),
    (
        "<script>-a ** b; (</script>",
        Some(("js_parse_error", "Unexpected token", 11, 11)),
    ),
    (
        "<script>x = new.target + ;</script>",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block",
            12,
            12,
        )),
    ),
    (
        "<script>x = import.meta.x + import.foo + ;</script>",
        Some((
            "js_parse_error",
            "The only valid meta property for import is 'import.meta'",
            35,
            35,
        )),
    ),
    (
        "<script>let a = 1;\nlet = ;\n</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 19, 19)),
    ),
    (
        "<script>let a = 1;\nx = 08 + ;\nlet b;</script>",
        Some(("js_parse_error", "Invalid number", 23, 23)),
    ),
    (
        "<script>function f() {\n  yield = ;\n}</script>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 25, 25)),
    ),
    (
        "<script lang=\"ts\">let = ;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 18, 18)),
    ),
    (
        "<script lang=\"ts\">let x: number = 08 + ;</script>",
        Some(("js_parse_error", "Invalid number", 34, 34)),
    ),
    (
        "<script lang=\"ts\">function f(this: any, a, a) { ( }</script>",
        Some(("js_parse_error", "Argument name clash", 43, 43)),
    ),
    (
        "<script lang=\"ts\">abstract class A { abstract x = 1; y( }</script>",
        Some((
            "js_parse_error",
            "Property 'x' cannot have an initializer because it is marked abstract.",
            48,
            48,
        )),
    ),
    (
        "<p>{let + }</p>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 4, 4)),
    ),
    (
        "<p>{yield ? }</p>",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 4, 4)),
    ),
    (
        "<p>{f(static, }</p>",
        Some(("js_parse_error", "The keyword 'static' is reserved", 6, 6)),
    ),
    (
        "<p>{08 + }</p>",
        Some(("js_parse_error", "Invalid number", 4, 4)),
    ),
    (
        "<p>{\"\\08\" + }</p>",
        Some(("js_parse_error", "Octal literal in strict mode", 5, 5)),
    ),
    (
        "<p>{[package, }</p>",
        Some(("js_parse_error", "The keyword 'package' is reserved", 5, 5)),
    ),
    (
        "<input value=\"{let + }\">",
        Some(("js_parse_error", "The keyword 'let' is reserved", 15, 15)),
    ),
    (
        "{#if let + }{/if}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 5, 5)),
    ),
    (
        "{#each let as x, }{/each}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 7, 7)),
    ),
    (
        "{@const a = let + }",
        Some(("js_parse_error", "The keyword 'let' is reserved", 12, 12)),
    ),
    (
        "{@html yield + }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 7, 7)),
    ),
    (
        "<p>{(a, a) => { ( }}</p>",
        Some(("js_parse_error", "Argument name clash", 8, 8)),
    ),
    (
        "<p>{function (a, a) {} + }</p>",
        Some(("js_parse_error", "Argument name clash", 17, 17)),
    ),
    (
        "<p>{delete x + }</p>",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            4,
            4,
        )),
    ),
    (
        "<p>{arguments = }</p>",
        Some((
            "js_parse_error",
            "Assigning to arguments in strict mode",
            4,
            4,
        )),
    ),
    (
        "<p>{eval++ + }</p>",
        Some(("js_parse_error", "Assigning to eval in strict mode", 4, 4)),
    ),
    (
        "<script>x = 1__0;</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            14,
            14,
        )),
    ),
    (
        "<script>x = 1__0 + 0;</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            14,
            14,
        )),
    ),
    (
        "<script>x = 0b12;</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>class A { constructor(){} constructor(){} foo(){} }</script>",
        Some((
            "js_parse_error",
            "Duplicate constructor in the same class",
            34,
            34,
        )),
    ),
    (
        "<script>class A { #a; #a; foo(){} }</script>",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            22,
            22,
        )),
    ),
    (
        "<script>label: function f() {}</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>export default 1; export default 2;</script>",
        Some(("js_parse_error", "Duplicate export 'default'", 33, 33)),
    ),
    (
        "<script>let a = 1; export { a, a };</script>",
        Some(("js_parse_error", "Duplicate export 'a'", 31, 31)),
    ),
    (
        "<script>class A { constructor(){} constructor(){} foo( ) { } }</script>",
        Some((
            "js_parse_error",
            "Duplicate constructor in the same class",
            34,
            34,
        )),
    ),
    (
        "<script>label: async function f() {}</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>label: function* f() {}</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>label: class A {}</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>a: b: function f() {}</script>",
        Some(("js_parse_error", "Unexpected token", 14, 14)),
    ),
    (
        "<script>label: let x;</script>",
        Some(("js_parse_error", "The keyword 'let' is reserved", 15, 15)),
    ),
    (
        "<script>label: const x = 1;</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    ("<script>label: var x;</script>", None),
    (
        "<script>while (0) function f() {}</script>",
        Some(("js_parse_error", "Unexpected token", 18, 18)),
    ),
    ("<script>label: { function f() {} }</script>", None),
    (
        "<script>function g() { label: function f() {} }</script>",
        Some(("js_parse_error", "Unexpected token", 30, 30)),
    ),
    ("<script>label: async () => 1</script>", None),
    ("<script>label: async\nfunction f() {}</script>", None),
    (
        "<script>label: using x = y;</script>",
        Some(("js_parse_error", "Unexpected token", 21, 21)),
    ),
    (
        "<script>x = 0o78;</script>",
        Some(("js_parse_error", "Unexpected token", 15, 15)),
    ),
    (
        "<script>x = 0x1g;</script>",
        Some(("js_parse_error", "Identifier directly after number", 15, 15)),
    ),
    (
        "<script>x = 0b1a;</script>",
        Some(("js_parse_error", "Identifier directly after number", 15, 15)),
    ),
    (
        "<script>x = 12a;</script>",
        Some(("js_parse_error", "Identifier directly after number", 14, 14)),
    ),
    (
        "<script>x = 1.5e3x;</script>",
        Some(("js_parse_error", "Identifier directly after number", 17, 17)),
    ),
    (
        "<script>x = 1_ ;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            13,
            13,
        )),
    ),
    (
        "<script>x = 1__;</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            14,
            14,
        )),
    ),
    (
        "<script>x = 0x_1;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the first of digits",
            14,
            14,
        )),
    ),
    (
        "<script>x = 0x1__2;</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            16,
            16,
        )),
    ),
    (
        "<script>x = 1_000__0n;</script>",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            18,
            18,
        )),
    ),
    (
        "<script>x = 1._5;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the first of digits",
            14,
            14,
        )),
    ),
    (
        "<script>x = 1.5_;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            15,
            15,
        )),
    ),
    (
        "<script>x = 1e_5;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the first of digits",
            14,
            14,
        )),
    ),
    (
        "<script>x = 1e5_ + ;</script>",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            15,
            15,
        )),
    ),
    (
        "{#each let as x}{/each}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 7, 7)),
    ),
    (
        "{#each 08 as x, }{/each}",
        Some(("js_parse_error", "Invalid number", 7, 7)),
    ),
    (
        "{#each a as x, i (let + )}{/each}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 18, 18)),
    ),
    (
        "{#each a as x (08)}{/each}",
        Some(("js_parse_error", "Invalid number", 15, 15)),
    ),
    (
        "{#if let}{:else if 08 + }{/if}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 5, 5)),
    ),
    (
        "{#await let then }{/await}",
        Some(("js_parse_error", "The keyword 'let' is reserved", 8, 8)),
    ),
    (
        "{#key 08}{/key",
        Some(("js_parse_error", "Invalid number", 6, 6)),
    ),
    (
        "{#each yield as }{/each}",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 7, 7)),
    ),
    (
        "<script>if (a) using x = y;</script>",
        Some(("js_parse_error", "Unexpected token", 21, 21)),
    ),
    ("<script>label: using\nx = y;</script>", None),
    ("<script lang=\"ts\">x = <let>y;</script>", None),
    ("<script lang=\"ts\">let a: let = 1;</script>", None),
    ("<script lang=\"ts\">x = y as let;</script>", None),
    ("<script lang=\"ts\">x = <yield>y;</script>", None),
    ("<script lang=\"ts\">type T = let;</script>", None),
];

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(src, expected) in CASES {
        let warnings =
            svn_lint::lint_file(src, Path::new("T.svelte"), None, CompatFeatures::MODERN);
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
