//! acorn's errors for `<script>` contents, one input per place acorn's
//! source raises an error.
//!
//! The compiler reads a script with acorn (acorn-typescript when the
//! component is TypeScript) as strict-mode module code. The inputs below
//! were written from acorn's own `raise` sites, so that each message it
//! can produce in that mode is reached at least once. Each expectation is
//! what svelte 5.57.2's `compile()` throws: code, first line of the
//! message, byte offset in the file.
//!
//! svelte-check drops a parser error whose message contains "expected"
//! when a script has a `lang` attribute and the project configures no
//! preprocessor, so for the TypeScript rows those are `None`.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, script contents, the compiler's error)`.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "function* g(a = yield) {}",
        Some((
            "js_parse_error",
            "Yield expression cannot be a default value",
            25,
        )),
    ),
    (
        false,
        "async function f(a = await 1) {}",
        Some((
            "js_parse_error",
            "Await expression cannot be a default value",
            30,
        )),
    ),
    (
        false,
        "if (a) { import x from 'y' }",
        Some((
            "js_parse_error",
            "'import' and 'export' may only appear at the top level",
            18,
        )),
    ),
    (
        false,
        "if (a) { export const b = 1 }",
        Some((
            "js_parse_error",
            "'import' and 'export' may only appear at the top level",
            18,
        )),
    ),
    (
        false,
        "function f() { export const b = 1 }",
        Some((
            "js_parse_error",
            "'import' and 'export' may only appear at the top level",
            24,
        )),
    ),
    (
        false,
        "for (let.x of y);",
        Some(("js_parse_error", "The keyword 'let' is reserved", 14)),
    ),
    (
        false,
        "for (let of y);",
        Some(("js_parse_error", "Unexpected token", 21)),
    ),
    (
        false,
        "switch (a) { default: default: }",
        Some(("js_parse_error", "Multiple default clauses", 31)),
    ),
    (
        false,
        "switch (a) { default: break; case 1: break; default: }",
        Some(("js_parse_error", "Multiple default clauses", 53)),
    ),
    (
        false,
        "throw\n1",
        Some(("js_parse_error", "Illegal newline after throw", 14)),
    ),
    (
        false,
        "try {}",
        Some(("js_parse_error", "Missing catch or finally clause", 9)),
    ),
    (
        false,
        "try {} \nlet a",
        Some(("js_parse_error", "Missing catch or finally clause", 9)),
    ),
    (
        false,
        "class A { #constructor }",
        Some((
            "js_parse_error",
            "Classes can't have an element named '#constructor'",
            19,
        )),
    ),
    (
        false,
        "class A { #constructor() {} }",
        Some((
            "js_parse_error",
            "Classes can't have an element named '#constructor'",
            19,
        )),
    ),
    (
        false,
        "class A { async constructor() {} }",
        Some(("js_parse_error", "Constructor can't be an async method", 25)),
    ),
    (
        false,
        "class A { static prototype() {} }",
        Some((
            "js_parse_error",
            "Classes may not have a static property named prototype",
            26,
        )),
    ),
    (
        false,
        "class A { static get prototype() { return 1 } }",
        Some((
            "js_parse_error",
            "Classes may not have a static property named prototype",
            30,
        )),
    ),
    (
        false,
        "({ set a(...b) {} })",
        Some(("js_parse_error", "Setter cannot use rest params", 18)),
    ),
    (
        false,
        "class A { set a(...b) {} }",
        Some(("js_parse_error", "Setter cannot use rest params", 25)),
    ),
    (
        false,
        "class A { constructor = 1 }",
        Some((
            "js_parse_error",
            "Classes can't have a field named 'constructor'",
            19,
        )),
    ),
    (
        false,
        "class A { 'constructor' = 1 }",
        Some((
            "js_parse_error",
            "Classes can't have a field named 'constructor'",
            19,
        )),
    ),
    (
        false,
        "const a = 1; export { \"a\" }",
        Some((
            "js_parse_error",
            "A string literal cannot be used as an exported binding without `from`.",
            31,
        )),
    ),
    (
        false,
        "import a from 'b' with { x: '1', x: '2' }",
        Some(("js_parse_error", "Duplicate attribute key 'x'", 42)),
    ),
    (
        false,
        "const a = 1; export { a as \"\\uD800\" }",
        Some((
            "js_parse_error",
            "An export name cannot include a lone surrogate.",
            36,
        )),
    ),
    (
        false,
        "async function f() { let await }",
        Some((
            "js_parse_error",
            "Cannot use 'await' as identifier inside an async function",
            34,
        )),
    ),
    (
        false,
        "async function f() { await: 1 }",
        Some(("js_parse_error", "Unexpected token", 35)),
    ),
    (
        false,
        "async () => { var await }",
        Some((
            "js_parse_error",
            "Cannot use 'await' as identifier inside an async function",
            27,
        )),
    ),
    (
        false,
        "({ get a() { return 1 } } = 1)",
        Some((
            "js_parse_error",
            "Object pattern can't contain getter or setter",
            16,
        )),
    ),
    (
        false,
        "({ set a(v) {} } = 1)",
        Some((
            "js_parse_error",
            "Object pattern can't contain getter or setter",
            16,
        )),
    ),
    (
        false,
        "[...a = 1] = b",
        Some((
            "js_parse_error",
            "Rest elements cannot have a default value",
            13,
        )),
    ),
    (
        false,
        "({ ...a = 1 } = b)",
        Some((
            "js_parse_error",
            "Rest elements cannot have a default value",
            15,
        )),
    ),
    (
        false,
        "([...a = 1]) => 1",
        Some((
            "js_parse_error",
            "Rest elements cannot have a default value",
            14,
        )),
    ),
    (
        false,
        "[a += 1] = b",
        Some((
            "js_parse_error",
            "Only '=' operator can be used for specifying default value.",
            11,
        )),
    ),
    (
        false,
        "({ a: b += 1 } = c)",
        Some((
            "js_parse_error",
            "Only '=' operator can be used for specifying default value.",
            16,
        )),
    ),
    (
        false,
        "([a += 1]) => 1",
        Some((
            "js_parse_error",
            "Only '=' operator can be used for specifying default value.",
            12,
        )),
    ),
    (
        false,
        "(a.b) => 1",
        Some(("js_parse_error", "Assigning to rvalue", 10)),
    ),
    (
        false,
        "([a.b]) => 1",
        Some(("js_parse_error", "Assigning to rvalue", 11)),
    ),
    (
        false,
        "({ a: b.c }) => 1",
        Some(("js_parse_error", "Assigning to rvalue", 15)),
    ),
    (
        false,
        "((a)) => 1",
        Some(("js_parse_error", "Parenthesized pattern", 10)),
    ),
    (
        false,
        "([(a)]) => 1",
        Some(("js_parse_error", "Parenthesized pattern", 11)),
    ),
    (
        false,
        "class A { #a; m() { return 1 + #a in this } }",
        Some((
            "js_parse_error",
            "Private identifier can only be left side of binary expression",
            40,
        )),
    ),
    (
        false,
        "class A { #a; m() { return #a + 1 } }",
        Some(("js_parse_error", "Unexpected token", 39)),
    ),
    (
        false,
        "class A { #a; m() { delete this.#a } }",
        Some(("js_parse_error", "Private fields can not be deleted", 29)),
    ),
    (
        false,
        "class A { #a; m() { delete this?.#a } }",
        Some(("js_parse_error", "Private fields can not be deleted", 29)),
    ),
    (
        false,
        "class A { #a; m() { delete (this.#a) } }",
        Some(("js_parse_error", "Private fields can not be deleted", 29)),
    ),
    (
        false,
        "new a?.b()",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in the callee of new expressions",
            14,
        )),
    ),
    (
        false,
        "new a?.()",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in the callee of new expressions",
            14,
        )),
    ),
    (false, "new (a?.b)()", None),
    (
        false,
        "im\\u0070ort.meta",
        Some(("js_parse_error", "Escape sequence in keyword import", 9)),
    ),
    (
        false,
        "import.foo",
        Some((
            "js_parse_error",
            "The only valid meta property for import is 'import.meta'",
            16,
        )),
    ),
    (
        false,
        "import.m\\u0065ta",
        Some((
            "js_parse_error",
            "'import.meta' must not contain escaped characters",
            9,
        )),
    ),
    (
        false,
        "function f() { n\\u0065w.target }",
        Some(("js_parse_error", "Escape sequence in keyword new", 24)),
    ),
    (
        false,
        "function f() { new.foo }",
        Some((
            "js_parse_error",
            "The only valid meta property for new is 'new.target'",
            28,
        )),
    ),
    (
        false,
        "function f() { new.t\\u0061rget }",
        Some((
            "js_parse_error",
            "'new.target' must not contain escaped characters",
            24,
        )),
    ),
    (
        false,
        "class A extends B { constructor() { new super() } }",
        Some(("js_parse_error", "Invalid use of 'super'", 49)),
    ),
    (false, "class A extends B { m() { new super.x } }", None),
    (
        false,
        "class A extends B { m() { super } }",
        Some(("js_parse_error", "Unexpected token", 41)),
    ),
    (
        false,
        "0_1",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed in legacy octal numeric literals",
            10,
        )),
    ),
    (
        false,
        "01_2",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed in legacy octal numeric literals",
            11,
        )),
    ),
    (
        false,
        "0x_1",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the first of digits",
            11,
        )),
    ),
    (
        false,
        "1_",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            10,
        )),
    ),
    (
        false,
        "1__2",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            11,
        )),
    ),
    (
        false,
        "1_.5",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            10,
        )),
    ),
    (
        false,
        "0x",
        Some(("js_parse_error", "Expected number in radix 16", 11)),
    ),
    (
        false,
        "0b",
        Some(("js_parse_error", "Expected number in radix 2", 11)),
    ),
    (
        false,
        "0o",
        Some(("js_parse_error", "Expected number in radix 8", 11)),
    ),
    (
        false,
        "0b2",
        Some(("js_parse_error", "Expected number in radix 2", 11)),
    ),
    (
        false,
        "`\\u{`",
        Some((
            "js_parse_error",
            "Bad escape sequence in untagged template literal",
            10,
        )),
    ),
    (false, "a`\\u{`", None),
    (
        false,
        "\\x61",
        Some((
            "js_parse_error",
            "Expecting Unicode escape sequence \\uXXXX",
            10,
        )),
    ),
    (
        false,
        "a\\x",
        Some((
            "js_parse_error",
            "Expecting Unicode escape sequence \\uXXXX",
            11,
        )),
    ),
    (false, "import('a',)", None),
    (false, "import('a', {},)", None),
    (
        false,
        "import('a', {}, 1)",
        Some(("js_parse_error", "Unexpected token", 25)),
    ),
    (
        false,
        "import()",
        Some(("js_parse_error", "Unexpected token", 16)),
    ),
    (
        false,
        "import(...a)",
        Some(("js_parse_error", "Unexpected token", 16)),
    ),
    (
        false,
        "a: a: ;",
        Some(("js_parse_error", "Label 'a' is already declared", 12)),
    ),
    (
        false,
        "a: { a: ; }",
        Some(("js_parse_error", "Label 'a' is already declared", 14)),
    ),
    (
        false,
        "function f() { let let }",
        Some(("js_parse_error", "The keyword 'let' is reserved", 28)),
    ),
    (
        false,
        "label: function* g() {}",
        Some(("js_parse_error", "Unexpected token", 16)),
    ),
    (
        false,
        "if (a) function f() {}",
        Some(("js_parse_error", "Unexpected token", 16)),
    ),
    (
        false,
        "if (a) class A {}",
        Some(("js_parse_error", "Unexpected token", 16)),
    ),
    (
        false,
        "while (a) let\nb",
        Some(("js_parse_error", "The keyword 'let' is reserved", 19)),
    ),
    (
        false,
        "({ a = 1 })",
        Some((
            "js_parse_error",
            "Shorthand property assignments are valid only in destructuring patterns",
            14,
        )),
    ),
    (
        false,
        "({ a = 1 }).b",
        Some((
            "js_parse_error",
            "Shorthand property assignments are valid only in destructuring patterns",
            14,
        )),
    ),
    (
        false,
        "({ __proto__: 1, __proto__: 2 })",
        Some(("js_parse_error", "Redefinition of __proto__ property", 26)),
    ),
    (
        false,
        "({ __proto__: 1, __proto__: 2 } = a)",
        Some(("js_parse_error", "Assigning to rvalue", 23)),
    ),
    (
        false,
        "a?.b = 1",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            9,
        )),
    ),
    (
        false,
        "a?.b++",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            9,
        )),
    ),
    (
        false,
        "[a?.b] = c",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            10,
        )),
    ),
    (
        false,
        "for (a?.b of c);",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            14,
        )),
    ),
    (
        false,
        "for (let a = 1 of b);",
        Some((
            "js_parse_error",
            "for-of loop variable declaration may not have an initializer",
            14,
        )),
    ),
    (
        false,
        "for (var a = 1 in b);",
        Some((
            "js_parse_error",
            "for-in loop variable declaration may not have an initializer",
            14,
        )),
    ),
    (
        false,
        "for (let a, b of c);",
        Some(("js_parse_error", "Unexpected token", 23)),
    ),
    (
        false,
        "for (async of a);",
        Some(("js_parse_error", "Unexpected token", 23)),
    ),
    (
        false,
        "class A { constructor() {} constructor() {} }",
        Some((
            "js_parse_error",
            "Duplicate constructor in the same class",
            36,
        )),
    ),
    (
        false,
        "class A { get constructor() {} }",
        Some((
            "js_parse_error",
            "Constructor can't have get/set modifier",
            23,
        )),
    ),
    (
        false,
        "class A { *constructor() {} }",
        Some(("js_parse_error", "Constructor can't be a generator", 20)),
    ),
    (
        false,
        "class A { static #a; static #a }",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            37,
        )),
    ),
    (
        false,
        "class A { #a; get #a() {} }",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            27,
        )),
    ),
    (false, "class A { get #a() {} set #a(v) {} }", None),
    (
        false,
        "({ get a(b) {} })",
        Some(("js_parse_error", "getter should have no params", 17)),
    ),
    (
        false,
        "({ set a() {} })",
        Some(("js_parse_error", "setter should have exactly one param", 17)),
    ),
    (
        false,
        "({ set a(b, c) {} })",
        Some(("js_parse_error", "setter should have exactly one param", 17)),
    ),
    (
        false,
        "class A { get a(b) {} }",
        Some(("js_parse_error", "getter should have no params", 24)),
    ),
    (
        false,
        "function f(a, a) { 'use strict' }",
        Some(("js_parse_error", "Argument name clash", 23)),
    ),
    (
        false,
        "function f(a = 1) { 'use strict' }",
        Some((
            "js_parse_error",
            "Illegal 'use strict' directive in function with non-simple parameter list",
            9,
        )),
    ),
    (
        false,
        "break",
        Some(("js_parse_error", "Unsyntactic break", 9)),
    ),
    (
        false,
        "continue",
        Some(("js_parse_error", "Unsyntactic continue", 9)),
    ),
    (
        false,
        "a: while (1) { continue b }",
        Some(("js_parse_error", "Unsyntactic continue", 24)),
    ),
    (
        false,
        "while (1) { break a }",
        Some(("js_parse_error", "Unsyntactic break", 21)),
    ),
    (
        false,
        "return 1",
        Some(("js_parse_error", "'return' outside of function", 9)),
    ),
    (
        false,
        "function f() { yield 1 }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 24)),
    ),
    (
        false,
        "async function f() { function g() { await 1 } }",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            45,
        )),
    ),
    (
        false,
        "class A { x = arguments }",
        Some((
            "js_parse_error",
            "Cannot use 'arguments' in class field initializer",
            23,
        )),
    ),
    (
        false,
        "class A { static { await 1 } }",
        Some((
            "js_parse_error",
            "Cannot use await in class static initialization block",
            28,
        )),
    ),
    (
        false,
        "class A { static { return } }",
        Some(("js_parse_error", "'return' outside of function", 28)),
    ),
    (
        false,
        "class A { static { arguments } }",
        Some((
            "js_parse_error",
            "Cannot use arguments in class static initialization block",
            28,
        )),
    ),
    (
        false,
        "var a; let a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            20,
        )),
    ),
    (
        false,
        "let a; var a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            20,
        )),
    ),
    (
        false,
        "function a() {} let a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            29,
        )),
    ),
    (
        false,
        "let a; function a() {}",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            25,
        )),
    ),
    (
        false,
        "class A {} class A {}",
        Some((
            "js_parse_error",
            "Identifier 'A' has already been declared",
            26,
        )),
    ),
    (
        false,
        "let a; { var a }",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            22,
        )),
    ),
    (
        false,
        "try {} catch (e) { let e }",
        Some((
            "js_parse_error",
            "Identifier 'e' has already been declared",
            32,
        )),
    ),
    (
        false,
        "try {} catch ([e, e]) {}",
        Some((
            "js_parse_error",
            "Identifier 'e' has already been declared",
            27,
        )),
    ),
    (
        false,
        "export default 1; export default 2",
        Some(("js_parse_error", "Duplicate export 'default'", 34)),
    ),
    (
        false,
        "export const a = 1; export { a }",
        Some(("js_parse_error", "Duplicate export 'a'", 38)),
    ),
    (false, "export { b }", None),
    (
        false,
        "import a from 'x'; import a from 'y'",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            35,
        )),
    ),
    (
        false,
        "import { a, a } from 'x'",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            21,
        )),
    ),
    (
        false,
        "let a; import a from 'x'",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            23,
        )),
    ),
    (
        false,
        "'\\08'",
        Some(("js_parse_error", "Octal literal in strict mode", 10)),
    ),
    (
        false,
        "'\\9'",
        Some(("js_parse_error", "Invalid escape sequence", 11)),
    ),
    (
        false,
        "\"\\u{110000}\"",
        Some(("js_parse_error", "Code point out of bounds", 13)),
    ),
    (
        false,
        "/a/gg",
        Some(("js_parse_error", "Duplicate regular expression flag", 10)),
    ),
    (
        false,
        "/(/",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(/: Unterminated group",
            10,
        )),
    ),
    (
        false,
        "/a/x",
        Some(("js_parse_error", "Invalid regular expression flag", 10)),
    ),
    (
        false,
        "/(?<a>x)(?<a>y)/",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<a>x)(?<a>y)/: Duplicate capture group name",
            10,
        )),
    ),
    (
        false,
        "var \\u{61}wait",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            13,
        )),
    ),
    (false, "({ a: 1, get a() { return 1 } })", None),
    (false, "x = { a, ...b } = c", None),
    (
        false,
        "({ ...{ a } } = b)",
        Some(("js_parse_error", "Unexpected token", 15)),
    ),
    (
        false,
        "({ ...[a] } = b)",
        Some(("js_parse_error", "Unexpected token", 15)),
    ),
    (
        false,
        "({ ...a, } = b)",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element",
            16,
        )),
    ),
    (
        false,
        "async (a = await 1) => 1",
        Some((
            "js_parse_error",
            "Await expression cannot be a default value",
            20,
        )),
    ),
    (
        false,
        "async function f() { (a = await 1) => 1 }",
        Some((
            "js_parse_error",
            "Await expression cannot be a default value",
            35,
        )),
    ),
    (
        false,
        "function* g() { (a = yield) => 1 }",
        Some((
            "js_parse_error",
            "Yield expression cannot be a default value",
            30,
        )),
    ),
    (
        false,
        "async (await) => 1",
        Some(("js_parse_error", "Unexpected token", 21)),
    ),
    (
        false,
        "async function f() { async (await) => 1 }",
        Some(("js_parse_error", "Unexpected token", 42)),
    ),
    (
        false,
        "(async)\n=> 1",
        Some(("js_parse_error", "Unexpected token", 17)),
    ),
    (
        false,
        "a\n=> 1",
        Some(("js_parse_error", "Unexpected token", 11)),
    ),
    (
        false,
        "a = () => {}\n()",
        Some(("js_parse_error", "Unexpected token", 23)),
    ),
    (
        false,
        "() => {} + 1",
        Some(("js_parse_error", "Unexpected token", 18)),
    ),
    (
        false,
        "({ a }) = 1",
        Some(("js_parse_error", "Assigning to rvalue", 9)),
    ),
    (
        false,
        "[a] = 1; ([a]) = 1",
        Some(("js_parse_error", "Assigning to rvalue", 18)),
    ),
    (
        false,
        "(a, b) = 1",
        Some(("js_parse_error", "Assigning to rvalue", 10)),
    ),
    (
        false,
        "(a = 1) = 2",
        Some(("js_parse_error", "Assigning to rvalue", 10)),
    ),
    (
        false,
        "a++ = 1",
        Some(("js_parse_error", "Assigning to rvalue", 9)),
    ),
    (
        false,
        "++a++",
        Some(("js_parse_error", "Assigning to rvalue", 11)),
    ),
    (
        false,
        "a = b ? c : d => 1 : 2",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    (
        false,
        "new.target",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block",
            9,
        )),
    ),
    (false, "function f() { new.target }", None),
    (
        false,
        "() => new.target",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block",
            15,
        )),
    ),
    (false, "class A { x = new.target }", None),
    (
        false,
        "super()",
        Some(("js_parse_error", "'super' keyword outside a method", 9)),
    ),
    (
        false,
        "class A { constructor() { super() } }",
        Some((
            "js_parse_error",
            "super() call outside constructor of a subclass",
            35,
        )),
    ),
    (
        false,
        "class A extends B { m() { super() } }",
        Some((
            "js_parse_error",
            "super() call outside constructor of a subclass",
            35,
        )),
    ),
    (false, "class A extends B { x = super.y }", None),
    (false, "({ m() { super.x } })", None),
    (
        false,
        "({ m: function() { super.x } })",
        Some(("js_parse_error", "'super' keyword outside a method", 28)),
    ),
    (
        false,
        "function f() { super.x }",
        Some(("js_parse_error", "'super' keyword outside a method", 24)),
    ),
    (
        false,
        "arguments",
        Some((
            "invalid_arguments_usage",
            "The arguments keyword cannot be used within the template or at the top level of a component",
            9,
        )),
    ),
    (
        false,
        "eval = 1",
        Some(("js_parse_error", "Assigning to eval in strict mode", 9)),
    ),
    (
        false,
        "arguments++",
        Some(("js_parse_error", "Assigning to arguments in strict mode", 9)),
    ),
    (
        false,
        "({ eval } = 1)",
        Some(("js_parse_error", "Assigning to eval in strict mode", 12)),
    ),
    (
        false,
        "function eval() {}",
        Some(("js_parse_error", "Binding eval in strict mode", 18)),
    ),
    (
        false,
        "class eval {}",
        Some(("js_parse_error", "Binding eval in strict mode", 15)),
    ),
    (
        false,
        "let eval",
        Some(("js_parse_error", "Binding eval in strict mode", 13)),
    ),
    (
        false,
        "try {} catch (eval) {}",
        Some(("js_parse_error", "Binding eval in strict mode", 23)),
    ),
    (
        false,
        "(eval) => 1",
        Some(("js_parse_error", "Binding eval in strict mode", 10)),
    ),
    (
        false,
        "function f(arguments) {}",
        Some(("js_parse_error", "Binding arguments in strict mode", 20)),
    ),
    (
        false,
        "delete a",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            9,
        )),
    ),
    (
        false,
        "delete (a)",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            9,
        )),
    ),
    (
        false,
        "delete ((a))",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            9,
        )),
    ),
    (false, "delete a.b", None),
    (
        false,
        "with (a) {}",
        Some(("js_parse_error", "'with' in strict mode", 9)),
    ),
    (false, "010", Some(("js_parse_error", "Invalid number", 9))),
    (false, "08", Some(("js_parse_error", "Invalid number", 9))),
    (false, "0.0_1", None),
    (
        false,
        "1n.5",
        Some(("js_parse_error", "Unexpected token", 11)),
    ),
    (false, "1e", Some(("js_parse_error", "Invalid number", 9))),
    (false, "1e+", Some(("js_parse_error", "Invalid number", 9))),
    (
        false,
        "0xg",
        Some(("js_parse_error", "Expected number in radix 16", 11)),
    ),
    (
        false,
        "'a\nb'",
        Some(("js_parse_error", "Unterminated string constant", 9)),
    ),
    (
        false,
        "`${a`",
        Some(("js_parse_error", "Unterminated template", 14)),
    ),
    (
        false,
        "/a",
        Some(("js_parse_error", "Unterminated regular expression", 10)),
    ),
    (
        false,
        "/a\n/",
        Some(("js_parse_error", "Unterminated regular expression", 10)),
    ),
    (
        false,
        "a ? b",
        Some(("js_parse_error", "Unexpected token", 15)),
    ),
    (
        false,
        "@dec class A {}",
        Some(("js_parse_error", "Unexpected character '@'", 9)),
    ),
    (
        false,
        "class A { @dec m() {} }",
        Some(("js_parse_error", "Unexpected character '@'", 19)),
    ),
    (
        false,
        "a => { yield 1 }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 16)),
    ),
    (
        false,
        "function* g() { yield\n* 1 }",
        Some(("js_parse_error", "Unexpected token", 31)),
    ),
    (
        false,
        "function* g() { function h() { yield 1 } }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 40)),
    ),
    (
        false,
        "function* g(yield) {}",
        Some((
            "js_parse_error",
            "Cannot use 'yield' as identifier inside a generator",
            21,
        )),
    ),
    (
        false,
        "function* yield() {}",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 19)),
    ),
    (
        false,
        "async function await() {}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            24,
        )),
    ),
    (
        false,
        "let yield",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 13)),
    ),
    (
        false,
        "yield: 1",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 9)),
    ),
    (
        false,
        "let static",
        Some(("js_parse_error", "The keyword 'static' is reserved", 13)),
    ),
    (
        false,
        "let implements",
        Some(("js_parse_error", "The keyword 'implements' is reserved", 13)),
    ),
    (
        false,
        "let enum",
        Some(("js_parse_error", "The keyword 'enum' is reserved", 13)),
    ),
    (
        false,
        "enum",
        Some(("js_parse_error", "The keyword 'enum' is reserved", 9)),
    ),
    (
        false,
        "let await",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            13,
        )),
    ),
    (
        false,
        "await: 1",
        Some(("js_parse_error", "Unexpected token", 14)),
    ),
    (false, "var async; async\nfunction f() {}", None),
    (
        false,
        "a ?? b && c",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            16,
        )),
    ),
    (
        false,
        "a && b ?? c",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            16,
        )),
    ),
    (false, "(a && b) ?? c", None),
    (false, "a ** -b", None),
    (
        false,
        "-a ** b",
        Some(("js_parse_error", "Unexpected token", 12)),
    ),
    (false, "(-a) ** b", None),
    (
        false,
        "typeof a ** b",
        Some(("js_parse_error", "Unexpected token", 18)),
    ),
    (
        false,
        "await a ** b",
        Some(("js_parse_error", "Unexpected token", 17)),
    ),
    (
        false,
        "async function f() { await a ** b }",
        Some(("js_parse_error", "Unexpected token", 38)),
    ),
    (
        false,
        "#a in b",
        Some(("js_parse_error", "Unexpected token", 9)),
    ),
    (
        false,
        "class A { m() { #a in b } }",
        Some((
            "js_parse_error",
            "Private field '#a' must be declared in an enclosing class",
            25,
        )),
    ),
    (
        false,
        "class A { m() { this.#a } }",
        Some((
            "js_parse_error",
            "Private field '#a' must be declared in an enclosing class",
            30,
        )),
    ),
    (
        false,
        "class A { #a; static m() { class B { m() { this.#a } } } }",
        None,
    ),
    (
        false,
        "this.#a",
        Some((
            "js_parse_error",
            "Private field '#a' must be declared in an enclosing class",
            14,
        )),
    ),
    (
        false,
        "a.#b.c",
        Some((
            "js_parse_error",
            "Private field '#b' must be declared in an enclosing class",
            11,
        )),
    ),
    (
        false,
        "class A { #a; #a }",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            23,
        )),
    ),
    (
        false,
        "class A { #a; m() { this.#b } }",
        Some((
            "js_parse_error",
            "Private field '#b' must be declared in an enclosing class",
            34,
        )),
    ),
    (
        false,
        "class A { m() { class B { #b } this.#b } }",
        Some((
            "js_parse_error",
            "Private field '#b' must be declared in an enclosing class",
            45,
        )),
    ),
    (
        true,
        "function* g(a = yield) {}",
        Some((
            "js_parse_error",
            "Yield expression cannot be a default value",
            35,
        )),
    ),
    (
        true,
        "async function f(a = await 1) {}",
        Some((
            "js_parse_error",
            "Await expression cannot be a default value",
            40,
        )),
    ),
    (
        true,
        "if (a) { import x from 'y' }",
        Some((
            "js_parse_error",
            "'import' and 'export' may only appear at the top level",
            28,
        )),
    ),
    (
        true,
        "if (a) { export const b = 1 }",
        Some((
            "js_parse_error",
            "'import' and 'export' may only appear at the top level",
            28,
        )),
    ),
    (
        true,
        "function f() { export const b = 1 }",
        Some((
            "js_parse_error",
            "'import' and 'export' may only appear at the top level",
            34,
        )),
    ),
    (
        true,
        "for (let.x of y);",
        Some(("js_parse_error", "The keyword 'let' is reserved", 24)),
    ),
    (true, "for (let of y);", None),
    (
        true,
        "switch (a) { default: default: }",
        Some(("js_parse_error", "Multiple default clauses", 41)),
    ),
    (
        true,
        "switch (a) { default: break; case 1: break; default: }",
        Some(("js_parse_error", "Multiple default clauses", 63)),
    ),
    (
        true,
        "throw\n1",
        Some(("js_parse_error", "Illegal newline after throw", 24)),
    ),
    (
        true,
        "try {}",
        Some(("js_parse_error", "Missing catch or finally clause", 19)),
    ),
    (
        true,
        "try {} \nlet a",
        Some(("js_parse_error", "Missing catch or finally clause", 19)),
    ),
    (
        true,
        "class A { #constructor }",
        Some((
            "js_parse_error",
            "Classes can't have an element named '#constructor'",
            29,
        )),
    ),
    (
        true,
        "class A { #constructor() {} }",
        Some((
            "js_parse_error",
            "Classes can't have an element named '#constructor'",
            29,
        )),
    ),
    (
        true,
        "class A { async constructor() {} }",
        Some(("js_parse_error", "Constructor can't be an async method", 35)),
    ),
    (
        true,
        "class A { static prototype() {} }",
        Some((
            "js_parse_error",
            "Classes may not have a static property named prototype",
            36,
        )),
    ),
    (
        true,
        "class A { static get prototype() { return 1 } }",
        Some((
            "js_parse_error",
            "Classes may not have a static property named prototype",
            40,
        )),
    ),
    (
        true,
        "({ set a(...b) {} })",
        Some(("js_parse_error", "Setter cannot use rest params", 28)),
    ),
    (
        true,
        "class A { set a(...b) {} }",
        Some(("js_parse_error", "Setter cannot use rest params", 35)),
    ),
    (
        true,
        "class A { constructor = 1 }",
        Some((
            "js_parse_error",
            "Classes can't have a field named 'constructor'",
            29,
        )),
    ),
    (
        true,
        "class A { 'constructor' = 1 }",
        Some((
            "js_parse_error",
            "Classes can't have a field named 'constructor'",
            29,
        )),
    ),
    (
        true,
        "const a = 1; export { \"a\" }",
        Some((
            "js_parse_error",
            "A string literal cannot be used as an exported binding without `from`.",
            41,
        )),
    ),
    (
        true,
        "import a from 'b' with { x: '1', x: '2' }",
        Some(("js_parse_error", "Duplicated key in attributes", 58)),
    ),
    (
        true,
        "const a = 1; export { a as \"\\uD800\" }",
        Some((
            "js_parse_error",
            "An export name cannot include a lone surrogate.",
            46,
        )),
    ),
    (
        true,
        "async function f() { let await }",
        Some((
            "js_parse_error",
            "Cannot use 'await' as identifier inside an async function",
            44,
        )),
    ),
    (true, "async function f() { await: 1 }", None),
    (
        true,
        "async () => { var await }",
        Some((
            "js_parse_error",
            "Cannot use 'await' as identifier inside an async function",
            37,
        )),
    ),
    (
        true,
        "({ get a() { return 1 } } = 1)",
        Some((
            "js_parse_error",
            "Object pattern can't contain getter or setter",
            26,
        )),
    ),
    (
        true,
        "({ set a(v) {} } = 1)",
        Some((
            "js_parse_error",
            "Object pattern can't contain getter or setter",
            26,
        )),
    ),
    (
        true,
        "[...a = 1] = b",
        Some((
            "js_parse_error",
            "Rest elements cannot have a default value",
            23,
        )),
    ),
    (
        true,
        "({ ...a = 1 } = b)",
        Some((
            "js_parse_error",
            "Rest elements cannot have a default value",
            25,
        )),
    ),
    (
        true,
        "([...a = 1]) => 1",
        Some((
            "js_parse_error",
            "Rest elements cannot have a default value",
            24,
        )),
    ),
    (
        true,
        "[a += 1] = b",
        Some((
            "js_parse_error",
            "Only '=' operator can be used for specifying default value.",
            21,
        )),
    ),
    (
        true,
        "({ a: b += 1 } = c)",
        Some((
            "js_parse_error",
            "Only '=' operator can be used for specifying default value.",
            26,
        )),
    ),
    (
        true,
        "([a += 1]) => 1",
        Some((
            "js_parse_error",
            "Only '=' operator can be used for specifying default value.",
            22,
        )),
    ),
    (
        true,
        "(a.b) => 1",
        Some(("js_parse_error", "Binding member expression", 20)),
    ),
    (
        true,
        "([a.b]) => 1",
        Some(("js_parse_error", "Binding member expression", 21)),
    ),
    (
        true,
        "({ a: b.c }) => 1",
        Some(("js_parse_error", "Binding member expression", 25)),
    ),
    (
        true,
        "((a)) => 1",
        Some(("js_parse_error", "Parenthesized pattern", 20)),
    ),
    (
        true,
        "([(a)]) => 1",
        Some(("js_parse_error", "Parenthesized pattern", 21)),
    ),
    (
        true,
        "class A { #a; m() { return 1 + #a in this } }",
        Some((
            "js_parse_error",
            "Private identifier can only be left side of binary expression",
            50,
        )),
    ),
    (true, "class A { #a; m() { return #a + 1 } }", None),
    (
        true,
        "class A { #a; m() { delete this.#a } }",
        Some(("js_parse_error", "Private fields can not be deleted", 39)),
    ),
    (
        true,
        "class A { #a; m() { delete this?.#a } }",
        Some(("js_parse_error", "Private fields can not be deleted", 39)),
    ),
    (
        true,
        "class A { #a; m() { delete (this.#a) } }",
        Some(("js_parse_error", "Private fields can not be deleted", 39)),
    ),
    (
        true,
        "new a?.b()",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in the callee of new expressions",
            24,
        )),
    ),
    (
        true,
        "new a?.()",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in the callee of new expressions",
            24,
        )),
    ),
    (true, "new (a?.b)()", None),
    (
        true,
        "im\\u0070ort.meta",
        Some(("js_parse_error", "Escape sequence in keyword import", 19)),
    ),
    (
        true,
        "import.foo",
        Some((
            "js_parse_error",
            "The only valid meta property for import is 'import.meta'",
            26,
        )),
    ),
    (
        true,
        "import.m\\u0065ta",
        Some((
            "js_parse_error",
            "'import.meta' must not contain escaped characters",
            19,
        )),
    ),
    (
        true,
        "function f() { n\\u0065w.target }",
        Some(("js_parse_error", "Escape sequence in keyword new", 34)),
    ),
    (
        true,
        "function f() { new.foo }",
        Some((
            "js_parse_error",
            "The only valid meta property for new is 'new.target'",
            38,
        )),
    ),
    (
        true,
        "function f() { new.t\\u0061rget }",
        Some((
            "js_parse_error",
            "'new.target' must not contain escaped characters",
            34,
        )),
    ),
    (
        true,
        "class A extends B { constructor() { new super() } }",
        None,
    ),
    (true, "class A extends B { m() { new super.x } }", None),
    (true, "class A extends B { m() { super } }", None),
    (
        true,
        "0_1",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed in legacy octal numeric literals",
            20,
        )),
    ),
    (
        true,
        "01_2",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed in legacy octal numeric literals",
            21,
        )),
    ),
    (
        true,
        "0x_1",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the first of digits",
            21,
        )),
    ),
    (
        true,
        "1_",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            20,
        )),
    ),
    (
        true,
        "1__2",
        Some((
            "js_parse_error",
            "Numeric separator must be exactly one underscore",
            21,
        )),
    ),
    (
        true,
        "1_.5",
        Some((
            "js_parse_error",
            "Numeric separator is not allowed at the last of digits",
            20,
        )),
    ),
    (
        true,
        "0x",
        Some(("js_parse_error", "Expected number in radix 16", 21)),
    ),
    (
        true,
        "0b",
        Some(("js_parse_error", "Expected number in radix 2", 21)),
    ),
    (
        true,
        "0o",
        Some(("js_parse_error", "Expected number in radix 8", 21)),
    ),
    (
        true,
        "0b2",
        Some(("js_parse_error", "Expected number in radix 2", 21)),
    ),
    (
        true,
        "`\\u{`",
        Some((
            "js_parse_error",
            "Bad escape sequence in untagged template literal",
            20,
        )),
    ),
    (true, "a`\\u{`", None),
    (
        true,
        "\\x61",
        Some((
            "js_parse_error",
            "Expecting Unicode escape sequence \\uXXXX",
            20,
        )),
    ),
    (
        true,
        "a\\x",
        Some((
            "js_parse_error",
            "Expecting Unicode escape sequence \\uXXXX",
            21,
        )),
    ),
    (true, "import('a',)", None),
    (true, "import('a', {},)", None),
    (true, "import('a', {}, 1)", None),
    (true, "import()", None),
    (true, "import(...a)", None),
    (
        true,
        "a: a: ;",
        Some(("js_parse_error", "Label 'a' is already declared", 22)),
    ),
    (
        true,
        "a: { a: ; }",
        Some(("js_parse_error", "Label 'a' is already declared", 24)),
    ),
    (
        true,
        "function f() { let let }",
        Some(("js_parse_error", "The keyword 'let' is reserved", 38)),
    ),
    (true, "label: function* g() {}", None),
    (true, "if (a) function f() {}", None),
    (true, "if (a) class A {}", None),
    (
        true,
        "while (a) let\nb",
        Some(("js_parse_error", "The keyword 'let' is reserved", 29)),
    ),
    (
        true,
        "({ a = 1 })",
        Some((
            "js_parse_error",
            "Shorthand property assignments are valid only in destructuring patterns",
            24,
        )),
    ),
    (
        true,
        "({ a = 1 }).b",
        Some((
            "js_parse_error",
            "Shorthand property assignments are valid only in destructuring patterns",
            24,
        )),
    ),
    (
        true,
        "({ __proto__: 1, __proto__: 2 })",
        Some(("js_parse_error", "Redefinition of __proto__ property", 36)),
    ),
    (
        true,
        "({ __proto__: 1, __proto__: 2 } = a)",
        Some(("js_parse_error", "Assigning to rvalue", 33)),
    ),
    (
        true,
        "a?.b = 1",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            19,
        )),
    ),
    (
        true,
        "a?.b++",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            19,
        )),
    ),
    (
        true,
        "[a?.b] = c",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            20,
        )),
    ),
    (
        true,
        "for (a?.b of c);",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in left-hand side",
            24,
        )),
    ),
    (
        true,
        "for (let a = 1 of b);",
        Some((
            "js_parse_error",
            "for-of loop variable declaration may not have an initializer",
            24,
        )),
    ),
    (
        true,
        "for (var a = 1 in b);",
        Some((
            "js_parse_error",
            "for-in loop variable declaration may not have an initializer",
            24,
        )),
    ),
    (true, "for (let a, b of c);", None),
    (true, "for (async of a);", None),
    (
        true,
        "class A { constructor() {} constructor() {} }",
        Some((
            "js_parse_error",
            "Duplicate constructor in the same class",
            46,
        )),
    ),
    (
        true,
        "class A { get constructor() {} }",
        Some((
            "js_parse_error",
            "Constructor can't have get/set modifier",
            33,
        )),
    ),
    (
        true,
        "class A { *constructor() {} }",
        Some(("js_parse_error", "Constructor can't be a generator", 30)),
    ),
    (
        true,
        "class A { static #a; static #a }",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            47,
        )),
    ),
    (
        true,
        "class A { #a; get #a() {} }",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            37,
        )),
    ),
    (true, "class A { get #a() {} set #a(v) {} }", None),
    (
        true,
        "({ get a(b) {} })",
        Some(("js_parse_error", "getter should have no params", 27)),
    ),
    (
        true,
        "({ set a() {} })",
        Some(("js_parse_error", "setter should have exactly one param", 27)),
    ),
    (
        true,
        "({ set a(b, c) {} })",
        Some(("js_parse_error", "setter should have exactly one param", 27)),
    ),
    (
        true,
        "class A { get a(b) {} }",
        Some(("js_parse_error", "getter should have no params", 34)),
    ),
    (
        true,
        "function f(a, a) { 'use strict' }",
        Some(("js_parse_error", "Argument name clash", 33)),
    ),
    // acorn-typescript lets this one pass.
    (true, "function f(a = 1) { 'use strict' }", None),
    (
        true,
        "break",
        Some(("js_parse_error", "Unsyntactic break", 19)),
    ),
    (
        true,
        "continue",
        Some(("js_parse_error", "Unsyntactic continue", 19)),
    ),
    (
        true,
        "a: while (1) { continue b }",
        Some(("js_parse_error", "Unsyntactic continue", 34)),
    ),
    (
        true,
        "while (1) { break a }",
        Some(("js_parse_error", "Unsyntactic break", 31)),
    ),
    (
        true,
        "return 1",
        Some(("js_parse_error", "'return' outside of function", 19)),
    ),
    (
        true,
        "function f() { yield 1 }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 34)),
    ),
    (
        true,
        "async function f() { function g() { await 1 } }",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            55,
        )),
    ),
    (
        true,
        "class A { x = arguments }",
        Some((
            "js_parse_error",
            "Cannot use 'arguments' in class field initializer",
            33,
        )),
    ),
    (
        true,
        "class A { static { await 1 } }",
        Some((
            "js_parse_error",
            "Cannot use await in class static initialization block",
            38,
        )),
    ),
    (
        true,
        "class A { static { return } }",
        Some(("js_parse_error", "'return' outside of function", 38)),
    ),
    (
        true,
        "class A { static { arguments } }",
        Some((
            "js_parse_error",
            "Cannot use arguments in class static initialization block",
            38,
        )),
    ),
    (
        true,
        "var a; let a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            30,
        )),
    ),
    (
        true,
        "let a; var a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            30,
        )),
    ),
    (
        true,
        "function a() {} let a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            39,
        )),
    ),
    (
        true,
        "let a; function a() {}",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            35,
        )),
    ),
    (
        true,
        "class A {} class A {}",
        Some((
            "js_parse_error",
            "Identifier 'A' has already been declared",
            36,
        )),
    ),
    (
        true,
        "let a; { var a }",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            32,
        )),
    ),
    (
        true,
        "try {} catch (e) { let e }",
        Some((
            "js_parse_error",
            "Identifier 'e' has already been declared",
            42,
        )),
    ),
    (
        true,
        "try {} catch ([e, e]) {}",
        Some((
            "js_parse_error",
            "Identifier 'e' has already been declared",
            37,
        )),
    ),
    (
        true,
        "export default 1; export default 2",
        Some((
            "module_illegal_default_export",
            "A component cannot have a default export",
            19,
        )),
    ),
    (true, "export const a = 1; export { a }", None),
    (true, "export { b }", None),
    // An import clashes with an import only, and acorn-typescript says
    // so itself, with a full stop.
    (
        true,
        "import a from 'x'; import a from 'y'",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared.",
            45,
        )),
    ),
    // An import clashes with an import only, and acorn-typescript says
    // so itself, with a full stop.
    (
        true,
        "import { a, a } from 'x'",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared.",
            31,
        )),
    ),
    // It parses: an import is kept apart from the scope's other names.
    // The compiler then refuses the variable, which shares the name of
    // an import in the module's scope.
    (
        true,
        "let a; import a from 'x'",
        Some((
            "declaration_duplicate_module_import",
            "Cannot declare a variable with the same name as an import from `<script module>`",
            23,
        )),
    ),
    (
        true,
        "'\\08'",
        Some(("js_parse_error", "Octal literal in strict mode", 20)),
    ),
    (
        true,
        "'\\9'",
        Some(("js_parse_error", "Invalid escape sequence", 21)),
    ),
    (
        true,
        "\"\\u{110000}\"",
        Some(("js_parse_error", "Code point out of bounds", 23)),
    ),
    (
        true,
        "/a/gg",
        Some(("js_parse_error", "Duplicate regular expression flag", 20)),
    ),
    (
        true,
        "/(/",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(/: Unterminated group",
            20,
        )),
    ),
    (
        true,
        "/a/x",
        Some(("js_parse_error", "Invalid regular expression flag", 20)),
    ),
    (
        true,
        "/(?<a>x)(?<a>y)/",
        Some((
            "js_parse_error",
            "Invalid regular expression: /(?<a>x)(?<a>y)/: Duplicate capture group name",
            20,
        )),
    ),
    (
        true,
        "var \\u{61}wait",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            23,
        )),
    ),
    (true, "({ a: 1, get a() { return 1 } })", None),
    (true, "x = { a, ...b } = c", None),
    (true, "({ ...{ a } } = b)", None),
    (true, "({ ...[a] } = b)", None),
    (
        true,
        "({ ...a, } = b)",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element",
            26,
        )),
    ),
    (
        true,
        "async (a = await 1) => 1",
        Some((
            "js_parse_error",
            "Await expression cannot be a default value",
            30,
        )),
    ),
    (
        true,
        "async function f() { (a = await 1) => 1 }",
        Some((
            "js_parse_error",
            "Await expression cannot be a default value",
            45,
        )),
    ),
    (
        true,
        "function* g() { (a = yield) => 1 }",
        Some((
            "js_parse_error",
            "Yield expression cannot be a default value",
            40,
        )),
    ),
    (true, "async (await) => 1", None),
    (true, "async function f() { async (await) => 1 }", None),
    (true, "(async)\n=> 1", None),
    (true, "a\n=> 1", None),
    (true, "a = () => {}\n()", None),
    (true, "() => {} + 1", None),
    (
        true,
        "({ a }) = 1",
        Some(("js_parse_error", "Assigning to rvalue", 19)),
    ),
    (
        true,
        "[a] = 1; ([a]) = 1",
        Some(("js_parse_error", "Assigning to rvalue", 28)),
    ),
    (
        true,
        "(a, b) = 1",
        Some(("js_parse_error", "Assigning to rvalue", 20)),
    ),
    (
        true,
        "(a = 1) = 2",
        Some(("js_parse_error", "Assigning to rvalue", 20)),
    ),
    (
        true,
        "a++ = 1",
        Some(("js_parse_error", "Assigning to rvalue", 19)),
    ),
    (
        true,
        "++a++",
        Some(("js_parse_error", "Assigning to rvalue", 21)),
    ),
    (true, "a = b ? c : d => 1 : 2", None),
    (
        true,
        "new.target",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block",
            19,
        )),
    ),
    (true, "function f() { new.target }", None),
    (
        true,
        "() => new.target",
        Some((
            "js_parse_error",
            "'new.target' can only be used in functions and class static block",
            25,
        )),
    ),
    (true, "class A { x = new.target }", None),
    (
        true,
        "super()",
        Some(("js_parse_error", "'super' keyword outside a method", 19)),
    ),
    (
        true,
        "class A { constructor() { super() } }",
        Some((
            "js_parse_error",
            "super() call outside constructor of a subclass",
            45,
        )),
    ),
    (
        true,
        "class A extends B { m() { super() } }",
        Some((
            "js_parse_error",
            "super() call outside constructor of a subclass",
            45,
        )),
    ),
    (true, "class A extends B { x = super.y }", None),
    (true, "({ m() { super.x } })", None),
    (
        true,
        "({ m: function() { super.x } })",
        Some(("js_parse_error", "'super' keyword outside a method", 38)),
    ),
    (
        true,
        "function f() { super.x }",
        Some(("js_parse_error", "'super' keyword outside a method", 34)),
    ),
    (
        true,
        "arguments",
        Some((
            "invalid_arguments_usage",
            "The arguments keyword cannot be used within the template or at the top level of a component",
            19,
        )),
    ),
    (
        true,
        "eval = 1",
        Some(("js_parse_error", "Assigning to eval in strict mode", 19)),
    ),
    (
        true,
        "arguments++",
        Some((
            "js_parse_error",
            "Assigning to arguments in strict mode",
            19,
        )),
    ),
    (
        true,
        "function eval() {}",
        Some(("js_parse_error", "Binding eval in strict mode", 28)),
    ),
    (
        true,
        "class eval {}",
        Some(("js_parse_error", "Binding eval in strict mode", 25)),
    ),
    (
        true,
        "let eval",
        Some(("js_parse_error", "Binding eval in strict mode", 23)),
    ),
    (
        true,
        "try {} catch (eval) {}",
        Some(("js_parse_error", "Binding eval in strict mode", 33)),
    ),
    (
        true,
        "(eval) => 1",
        Some(("js_parse_error", "Binding eval in strict mode", 20)),
    ),
    (
        true,
        "function f(arguments) {}",
        Some(("js_parse_error", "Binding arguments in strict mode", 30)),
    ),
    (
        true,
        "delete a",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            19,
        )),
    ),
    (
        true,
        "delete (a)",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            19,
        )),
    ),
    (
        true,
        "delete ((a))",
        Some((
            "js_parse_error",
            "Deleting local variable in strict mode",
            19,
        )),
    ),
    (true, "delete a.b", None),
    (
        true,
        "with (a) {}",
        Some(("js_parse_error", "'with' in strict mode", 19)),
    ),
    (true, "010", Some(("js_parse_error", "Invalid number", 19))),
    (true, "08", Some(("js_parse_error", "Invalid number", 19))),
    (true, "0.0_1", None),
    (true, "1n.5", None),
    (true, "1e", Some(("js_parse_error", "Invalid number", 19))),
    (true, "1e+", Some(("js_parse_error", "Invalid number", 19))),
    (
        true,
        "0xg",
        Some(("js_parse_error", "Expected number in radix 16", 21)),
    ),
    (
        true,
        "'a\nb'",
        Some(("js_parse_error", "Unterminated string constant", 19)),
    ),
    (
        true,
        "`${a`",
        Some(("js_parse_error", "Unterminated template", 24)),
    ),
    (
        true,
        "/a",
        Some(("js_parse_error", "Unterminated regular expression", 20)),
    ),
    (
        true,
        "/a\n/",
        Some(("js_parse_error", "Unterminated regular expression", 20)),
    ),
    (true, "a ? b", None),
    (
        true,
        "@dec class A {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        true,
        "class A { @dec m() {} }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            29,
        )),
    ),
    (
        true,
        "a => { yield 1 }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 26)),
    ),
    (true, "function* g() { yield\n* 1 }", None),
    (
        true,
        "function* g() { function h() { yield 1 } }",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 50)),
    ),
    (
        true,
        "function* g(yield) {}",
        Some((
            "js_parse_error",
            "Cannot use 'yield' as identifier inside a generator",
            31,
        )),
    ),
    (
        true,
        "function* yield() {}",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 29)),
    ),
    (
        true,
        "async function await() {}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            34,
        )),
    ),
    (
        true,
        "let yield",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 23)),
    ),
    (
        true,
        "yield: 1",
        Some(("js_parse_error", "The keyword 'yield' is reserved", 19)),
    ),
    (
        true,
        "let static",
        Some(("js_parse_error", "The keyword 'static' is reserved", 23)),
    ),
    (
        true,
        "let implements",
        Some(("js_parse_error", "The keyword 'implements' is reserved", 23)),
    ),
    (
        true,
        "let enum",
        Some(("js_parse_error", "The keyword 'enum' is reserved", 23)),
    ),
    (true, "enum", None),
    (
        true,
        "let await",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function",
            23,
        )),
    ),
    (true, "await: 1", None),
    (true, "var async; async\nfunction f() {}", None),
    (
        true,
        "a ?? b && c",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            26,
        )),
    ),
    (
        true,
        "a && b ?? c",
        Some((
            "js_parse_error",
            "Logical expressions and coalesce expressions cannot be mixed. Wrap either by parentheses",
            26,
        )),
    ),
    (true, "(a && b) ?? c", None),
    (true, "a ** -b", None),
    (true, "-a ** b", None),
    (true, "(-a) ** b", None),
    (true, "typeof a ** b", None),
    (true, "async function f() { await a ** b }", None),
    (true, "#a in b", None),
    (
        true,
        "class A { m() { #a in b } }",
        Some((
            "js_parse_error",
            "Private field '#a' must be declared in an enclosing class",
            35,
        )),
    ),
    (
        true,
        "class A { m() { this.#a } }",
        Some((
            "js_parse_error",
            "Private field '#a' must be declared in an enclosing class",
            40,
        )),
    ),
    (
        true,
        "class A { #a; static m() { class B { m() { this.#a } } } }",
        None,
    ),
    (
        true,
        "this.#a",
        Some((
            "js_parse_error",
            "Private field '#a' must be declared in an enclosing class",
            24,
        )),
    ),
    (
        true,
        "a.#b.c",
        Some((
            "js_parse_error",
            "Private field '#b' must be declared in an enclosing class",
            21,
        )),
    ),
    (
        true,
        "class A { #a; #a }",
        Some((
            "js_parse_error",
            "Identifier '#a' has already been declared",
            33,
        )),
    ),
    (
        true,
        "class A { #a; m() { this.#b } }",
        Some((
            "js_parse_error",
            "Private field '#b' must be declared in an enclosing class",
            44,
        )),
    ),
    (
        true,
        "class A { m() { class B { #b } this.#b } }",
        Some((
            "js_parse_error",
            "Private field '#b' must be declared in an enclosing class",
            55,
        )),
    ),
];

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(ts, code, expected) in CASES {
        let lang = if ts { " lang=\"ts\"" } else { "" };
        let src = format!("<script{lang}>\n{code}\n</script>\n");
        let warnings = svn_lint::lint_file(
            &src,
            Path::new("t.svelte"),
            Some(true),
            CompatFeatures::MODERN,
        );
        let got = warnings.iter().find(|w| w.is_error).map(|w| {
            (
                w.code.as_str(),
                w.message.lines().next().unwrap_or_default(),
                w.range.start,
            )
        });
        if got != expected {
            failures.push(format!(
                "ts={ts} {code:?}\n  got:  {got:?}\n  want: {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
