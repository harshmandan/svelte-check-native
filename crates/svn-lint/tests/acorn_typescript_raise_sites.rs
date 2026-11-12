//! acorn-typescript's errors for `<script lang="ts">` contents, one
//! input or more per error in acorn-typescript's own table.
//!
//! The compiler reads a TypeScript component's scripts with
//! acorn-typescript. It rejects less than TypeScript does, but not
//! nothing: an abstract method with a body, an initializer in a declared
//! variable, an empty type parameter list, modifiers in the wrong place
//! and the like are errors there too, in its own words and at its own
//! positions. The inputs were written from the error table and the
//! `raise` calls in acorn-typescript 1.0.13, with valid TypeScript mixed
//! in so that nothing accepted gets reported. Each expectation is what
//! svelte 5.57.2's `compile()` throws: code, first line of the message,
//! byte offset in the file.
//!
//! The project is taken to configure a preprocessor, so that svelte-check
//! keeps the errors whose message contains "expected".

use std::path::Path;
use svn_lint::{CompatFeatures, LintOptions};

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(script contents, the compiler's error)`.
const CASES: &[(&str, Expected)] = &[
    (
        "abstract class A { abstract m() {} }",
        Some((
            "js_parse_error",
            "Method 'm' cannot have an implementation because it is marked abstract.",
            38,
        )),
    ),
    (
        "abstract class A { abstract x = 1 }",
        Some((
            "js_parse_error",
            "Property 'x' cannot have an initializer because it is marked abstract.",
            49,
        )),
    ),
    (
        "class A { get x(this: A) { return 1 } }",
        Some(("js_parse_error", "getter should have no params", 34)),
    ),
    (
        "class A { set x(this: A, v) {} }",
        Some(("js_parse_error", "setter should have exactly one param", 34)),
    ),
    ("class A { get x<T>() { return 1 } }", None),
    ("class A { set x<T>(v) {} }", None),
    (
        "class A { declare m() {} }",
        Some((
            "js_parse_error",
            "Class methods cannot have the 'declare' modifier.",
            29,
        )),
    ),
    (
        "class A { readonly m() {} }",
        Some((
            "js_parse_error",
            "Class methods cannot have the 'readonly' modifier.",
            29,
        )),
    ),
    (
        "declare const a = foo()",
        Some((
            "js_parse_error",
            "A 'const' initializer in an ambient context must be a string or numeric literal or literal enum reference.",
            37,
        )),
    ),
    ("declare const a = 1", None),
    (
        "declare const a: number = 1",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            45,
        )),
    ),
    (
        "declare let a = 1",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            35,
        )),
    ),
    (
        "declare var a = 1",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            35,
        )),
    ),
    (
        "class A { constructor<T>() {} }",
        Some((
            "js_parse_error",
            "Type parameters cannot appear on a constructor declaration.",
            40,
        )),
    ),
    (
        "class A { declare get x() { return 1 } }",
        Some((
            "js_parse_error",
            "Class methods cannot have the 'declare' modifier.",
            29,
        )),
    ),
    (
        "class A { declare set x(v) {} }",
        Some((
            "js_parse_error",
            "Class methods cannot have the 'declare' modifier.",
            29,
        )),
    ),
    (
        "class A { declare x = 1 }",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            39,
        )),
    ),
    (
        "declare class A { x = 1 }",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            39,
        )),
    ),
    (
        "declare function f() {}",
        Some((
            "js_parse_error",
            "An implementation cannot be declared in ambient contexts.",
            19,
        )),
    ),
    ("declare function f(): void", None),
    (
        "class A { public private x }",
        Some(("js_parse_error", "Accessibility modifier already seen.", 44)),
    ),
    (
        "class A { public public x }",
        Some(("js_parse_error", "Accessibility modifier already seen.", 43)),
    ),
    (
        "class A { readonly readonly x }",
        Some(("js_parse_error", "Duplicate modifier: 'readonly'.", 47)),
    ),
    (
        "class A { static static x }",
        Some(("js_parse_error", "Duplicate modifier: 'static'.", 43)),
    ),
    (
        "class A { declare declare x }",
        Some(("js_parse_error", "Duplicate modifier: 'declare'.", 45)),
    ),
    (
        "class A implements {}",
        Some(("js_parse_error", "'implements' list cannot be empty.", 38)),
    ),
    (
        "interface A extends {}",
        Some(("js_parse_error", "'extends' list cannot be empty.", 39)),
    ),
    (
        "class A extends B implements {}",
        Some(("js_parse_error", "'implements' list cannot be empty.", 48)),
    ),
    ("f<>()", Some(("js_parse_error", "Unexpected token", 21))),
    (
        "let a: Array<>",
        Some(("js_parse_error", "Type argument list cannot be empty.", 32)),
    ),
    (
        "new A<>()",
        Some(("js_parse_error", "Unexpected token", 25)),
    ),
    (
        "function f<>() {}",
        Some(("js_parse_error", "Type parameter list cannot be empty.", 31)),
    ),
    (
        "type A<> = 1",
        Some(("js_parse_error", "Type parameter list cannot be empty.", 28)),
    ),
    (
        "class A<> {}",
        Some(("js_parse_error", "Type parameter list cannot be empty.", 29)),
    ),
    (
        "interface A<> {}",
        Some(("js_parse_error", "Type parameter list cannot be empty.", 33)),
    ),
    (
        "export declare 1",
        Some((
            "js_parse_error",
            "'export declare' must be followed by an ambient declaration.",
            34,
        )),
    ),
    ("export declare let a: number", None),
    (
        "import type a = b.c",
        Some((
            "js_parse_error",
            "An import alias can not use 'import type'.",
            35,
        )),
    ),
    ("import a = b.c", None),
    ("abstract class A { declare abstract x }", None),
    (
        "class A { static abstract x }",
        Some((
            "js_parse_error",
            "'static' modifier cannot be used with 'abstract' modifier.",
            17,
        )),
    ),
    (
        "abstract class A { abstract static x }",
        Some((
            "js_parse_error",
            "'static' modifier cannot be used with 'abstract' modifier.",
            28,
        )),
    ),
    (
        "class A { declare override x }",
        Some((
            "js_parse_error",
            "'declare' modifier cannot be used with 'override' modifier.",
            18,
        )),
    ),
    (
        "class A { abstract [k: string]: any }",
        Some((
            "js_parse_error",
            "Index signatures cannot have the 'abstract' modifier.",
            29,
        )),
    ),
    (
        "class A { public [k: string]: any }",
        Some((
            "js_parse_error",
            "Index signatures cannot have an accessibility modifier ('public').",
            29,
        )),
    ),
    (
        "class A { declare [k: string]: any }",
        Some((
            "js_parse_error",
            "Index signatures cannot have the 'declare' modifier.",
            29,
        )),
    ),
    (
        "class A extends B { override [k: string]: any }",
        Some((
            "js_parse_error",
            "'override' modifier cannot appear on an index signature.",
            39,
        )),
    ),
    ("class A { static [k: string]: any }", None),
    ("class A { readonly [k: string]: any }", None),
    ("class A { [k: string]: any }", None),
    (
        "interface A { public x: number }",
        Some((
            "js_parse_error",
            "({ modifier }) => `'${modifier}' modifier cannot appear on a type member.`",
            40,
        )),
    ),
    (
        "type A = { static x: number }",
        Some((
            "js_parse_error",
            "({ modifier }) => `'${modifier}' modifier cannot appear on a type member.`",
            37,
        )),
    ),
    (
        "interface A { declare x: number }",
        Some((
            "js_parse_error",
            "({ modifier }) => `'${modifier}' modifier cannot appear on a type member.`",
            41,
        )),
    ),
    ("interface A { readonly x: number }", None),
    (
        "function f<public T>() {}",
        Some(("js_parse_error", "The keyword 'public' is reserved", 30)),
    ),
    (
        "function f<readonly T>() {}",
        Some(("js_parse_error", "Unexpected token", 39)),
    ),
    (
        "function f<in T>() {}",
        Some((
            "js_parse_error",
            "({ modifier }) => `'${modifier}' modifier can only appear on a type parameter of a class, interface or type alias.`",
            33,
        )),
    ),
    (
        "function f<out T>() {}",
        Some((
            "js_parse_error",
            "({ modifier }) => `'${modifier}' modifier can only appear on a type parameter of a class, interface or type alias.`",
            34,
        )),
    ),
    ("class A<in T> {}", None),
    ("class A<in out T> {}", None),
    ("function f<const T>() {}", None),
    (
        "class A { readonly public x }",
        Some((
            "js_parse_error",
            "'public' modifier must precede 'readonly' modifier.",
            19,
        )),
    ),
    (
        "class A { static public x }",
        Some((
            "js_parse_error",
            "'public' modifier must precede 'static' modifier.",
            17,
        )),
    ),
    (
        "class A extends B { override static x }",
        Some((
            "js_parse_error",
            "'static' modifier must precede 'override' modifier.",
            29,
        )),
    ),
    (
        "class A { readonly static x }",
        Some((
            "js_parse_error",
            "'static' modifier must precede 'readonly' modifier.",
            19,
        )),
    ),
    ("class A { readonly declare x }", None),
    ("class A { static readonly x }", None),
    (
        "a<b>.c",
        Some((
            "js_parse_error",
            "Invalid property access after an instantiation expression. You can either wrap the instantiation expression in parentheses, or delete the type arguments.",
            23,
        )),
    ),
    (
        "a<b>?.c",
        Some((
            "js_parse_error",
            "Invalid property access after an instantiation expression. You can either wrap the instantiation expression in parentheses, or delete the type arguments.",
            23,
        )),
    ),
    ("(a<b>).c", None),
    (
        "type A = [a.b: string]",
        Some((
            "js_parse_error",
            "Tuple members must be labeled with a simple identifier.",
            29,
        )),
    ),
    ("type A = [a: string]", None),
    (
        "interface {}",
        Some((
            "js_parse_error",
            "'interface' declarations must be followed by an identifier.",
            29,
        )),
    ),
    ("interface A {}", None),
    (
        "class A { abstract m(): void }",
        Some((
            "js_parse_error",
            "Abstract methods can only appear within an abstract class.",
            29,
        )),
    ),
    (
        "class A { abstract x: number }",
        Some((
            "js_parse_error",
            "Abstract methods can only appear within an abstract class.",
            29,
        )),
    ),
    (
        "type A = [a?, b]",
        Some((
            "js_parse_error",
            "A required element cannot follow an optional element.",
            33,
        )),
    ),
    (
        "type A = [x?: number, y: number]",
        Some((
            "js_parse_error",
            "A required element cannot follow an optional element.",
            41,
        )),
    ),
    ("type A = [x?: number, ...y: number[]]", None),
    (
        "class A { override m() {} }",
        Some((
            "js_parse_error",
            "This member cannot have an 'override' modifier because its containing class does not extend another class.",
            29,
        )),
    ),
    ("class A extends B { override m() {} }", None),
    (
        "function f({ a }?: any) {}",
        Some((
            "js_parse_error",
            "A binding pattern parameter cannot be optional in an implementation signature.",
            30,
        )),
    ),
    (
        "function f([a]?: any) {}",
        Some((
            "js_parse_error",
            "A binding pattern parameter cannot be optional in an implementation signature.",
            30,
        )),
    ),
    ("declare function f({ a }?: any): void", None),
    (
        "abstract class A { abstract #x }",
        Some((
            "js_parse_error",
            "Private elements cannot have the 'abstract' modifier.",
            38,
        )),
    ),
    (
        "class A { private #x }",
        Some((
            "js_parse_error",
            "Private elements cannot have an accessibility modifier ('private').",
            29,
        )),
    ),
    (
        "class A { public #x = 1 }",
        Some((
            "js_parse_error",
            "Private elements cannot have an accessibility modifier ('public').",
            29,
        )),
    ),
    (
        "class A { public #m() {} }",
        Some((
            "js_parse_error",
            "Private methods cannot have an accessibility modifier ('public').",
            29,
        )),
    ),
    (
        "interface A { readonly m(): void }",
        Some((
            "js_parse_error",
            "'readonly' modifier can only appear on a property declaration or index signature.",
            33,
        )),
    ),
    ("interface A { readonly x: number }", None),
    ("class A { set x(v?) {} }", None),
    (
        "class A { set x(...v) {} }",
        Some(("js_parse_error", "Setter cannot use rest params", 35)),
    ),
    ("class A { set x(v): void {} }", None),
    (
        "class A { get x(a) { return 1 } }",
        Some(("js_parse_error", "getter should have no params", 34)),
    ),
    (
        "class A { set x() {} }",
        Some(("js_parse_error", "setter should have exactly one param", 34)),
    ),
    (
        "class A { set x(a, b) {} }",
        Some(("js_parse_error", "setter should have exactly one param", 34)),
    ),
    (
        "class A { declare static {} }",
        Some((
            "js_parse_error",
            "Static class blocks cannot have any modifier.",
            45,
        )),
    ),
    (
        "class A { public static {} }",
        Some((
            "js_parse_error",
            "Static class blocks cannot have any modifier.",
            44,
        )),
    ),
    ("class A { static {} }", None),
    (
        "function f(a = 1: number) {}",
        Some((
            "js_parse_error",
            "Type annotations must come before default assignments, e.g. instead of `age = 25: number` use `age: number = 25`.",
            35,
        )),
    ),
    ("function f(a: number = 1) {}", None),
    (
        "import type a, { b } from 'c'",
        Some((
            "js_parse_error",
            "A type-only import can specify a default import or named bindings, but not both.",
            19,
        )),
    ),
    ("import type a from 'c'", None),
    ("import type { b } from 'c'", None),
    (
        "const a = 1; export type { type a }",
        Some((
            "js_parse_error",
            "The 'type' modifier cannot be used on a named export when 'export type' is used on its export statement.",
            46,
        )),
    ),
    (
        "import type { type a } from 'b'",
        Some((
            "js_parse_error",
            "The 'type' modifier cannot be used on a named import when 'import type' is used on its import statement.",
            33,
        )),
    ),
    ("import { type a } from 'b'", None),
    (
        "function f(public a) {}",
        Some(("js_parse_error", "The keyword 'public' is reserved", 30)),
    ),
    ("class A { m(private a) {} }", None),
    (
        "class A { constructor(private a) {} }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like accessibility modifiers on constructor parameters are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            41,
        )),
    ),
    (
        "class A { constructor(readonly a) {} }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like accessibility modifiers on constructor parameters are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            41,
        )),
    ),
    (
        "let a: readonly string",
        Some((
            "js_parse_error",
            "'readonly' type modifier is only permitted on array and tuple literal types.",
            26,
        )),
    ),
    ("let a: readonly string[]", None),
    ("let a: readonly [string]", None),
    ("({ a: 1 }: any)", None),
    ("let a = (b: number)", None),
    ("(a: number) => 1", None),
    ("f((a as any) = 1)", None),
    ("(a as b) => 1", None),
    ("(<any>a) => 1", None),
    (
        "((a as b)) => 1",
        Some(("js_parse_error", "Parenthesized pattern", 20)),
    ),
    (
        "let a: import(b)",
        Some((
            "js_parse_error",
            "Argument in a type import must be a string literal.",
            33,
        )),
    ),
    ("let a: import('b')", None),
    ("let a: typeof import('b')", None),
    (
        "class A { constructor(private { a }) {} }",
        Some((
            "js_parse_error",
            "A parameter property may not be declared using a binding pattern.",
            41,
        )),
    ),
    (
        "class A { constructor(private [a]) {} }",
        Some((
            "js_parse_error",
            "A parameter property may not be declared using a binding pattern.",
            41,
        )),
    ),
    (
        "class A { constructor(private ...a) {} }",
        Some(("js_parse_error", "Unexpected token", 49)),
    ),
    (
        "type A = (1) => void",
        Some(("js_parse_error", "Unexpected token", 32)),
    ),
    (
        "interface A { m(a.b): void }",
        Some(("js_parse_error", "Unexpected token", 36)),
    ),
    ("type A = (a?: number, b: number) => void", None),
    (
        "import a from 'b' with { x: 1 }",
        Some((
            "js_parse_error",
            "Only string is supported as an attribute value",
            48,
        )),
    ),
    ("import a from 'b' with { x: 'y' }", None),
    (
        "a?.b`c`",
        Some((
            "js_parse_error",
            "Optional chaining cannot appear in the tag of tagged template expressions",
            23,
        )),
    ),
    ("a?.b<c>`d`", None),
    (
        "class A { #x; #x }",
        Some((
            "js_parse_error",
            "Identifier '#x' has already been declared",
            33,
        )),
    ),
    (
        "let a; let a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            30,
        )),
    ),
    ("var a; var a", None),
    (
        "let a; var a",
        Some((
            "js_parse_error",
            "Identifier 'a' has already been declared",
            30,
        )),
    ),
    (
        "type A = 1; type A = 2",
        Some(("js_parse_error", "type 'A' has already been declared.", 36)),
    ),
    ("interface A {} interface A {}", None),
    ("type A = 1; interface A {}", None),
    ("type A = 1; let A", None),
    ("class A {} interface A {}", None),
    (
        "enum E { A } enum E { B }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    ("function f(): void; function f() {}", None),
    ("function f(): void; function g() {}", None),
    ("function f(a?, b) {}", None),
    ("function f(a?: number, b: number) {}", None),
    ("function f(a?: number = 1) {}", None),
    ("let a!: number", None),
    (
        "let a?: number",
        Some(("js_parse_error", "Unexpected token", 24)),
    ),
    ("let a!: number = 1", None),
    ("class A { x?: number; y!: number }", None),
    (
        "class A { x!?: number }",
        Some(("js_parse_error", "Unexpected token", 31)),
    ),
    (
        "class A { x?!: number }",
        Some(("js_parse_error", "Unexpected token", 31)),
    ),
    ("function f(this: A) {}", None),
    ("function f(a, this: A) {}", None),
    (
        "(this: A) => 1",
        Some(("js_parse_error", "Assigning to rvalue", 20)),
    ),
    (
        "enum E { A = 1, A }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "enum E { 1 }",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    (
        "enum E { 'a' }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "const enum E { A }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "enum E {} ",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "declare enum E { A }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    ("declare global {}", None),
    (
        "namespace A { export const b = 1 }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like namespaces with non-type nodes are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    ("module 'x' {}", None),
    ("declare module 'x' {}", None),
    ("declare module 'x'", None),
    ("import a = require('b')", None),
    ("export = a", None),
    ("export as namespace a", None),
    ("let a: unique symbol", None),
    ("function f(x): asserts x is string {}", None),
    ("function f(x): asserts x {}", None),
    ("function f(x): x is string { return true }", None),
    (
        "function f(x): \\u0061sserts x {}",
        Some(("js_parse_error", "Escape sequence in keyword asserts", 34)),
    ),
    (
        "class A { accessor x = 1 }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like accessor fields (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            29,
        )),
    ),
    (
        "class A { accessor accessor }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like accessor fields (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            29,
        )),
    ),
    ("type A<T> = T extends infer U ? U : never", None),
    ("type A = infer T", None),
    ("let a = <T>(b: T) => b", None),
    ("let a = <T,>(b: T) => b", None),
    ("let a = <T>b", None),
    ("let a = b!", None),
    ("let a = b!!", None),
    ("a! = 1", None),
    ("a!++", None),
    ("let a = b as c", None),
    ("let a = b satisfies c", None),
    ("let a = b as const", None),
    ("let a = <const>b", None),
    ("a as b = 1", None),
    ("(a as b) = 1", None),
    ("(a satisfies b) = 1", None),
    ("[a as b] = c", None),
    ("({ a: b as c } = d)", None),
    (
        "for (a as b of c);",
        Some((
            "js_parse_error",
            "Unexpected type cast in parameter position.",
            24,
        )),
    ),
    ("abstract class A {}", None),
    (
        "abstract class {}",
        Some(("js_parse_error", "Unexpected token", 34)),
    ),
    ("abstract interface A {}", None),
    (
        "abstract let a",
        Some(("js_parse_error", "Unexpected token", 19)),
    ),
    ("declare abstract class A {}", None),
    ("export abstract class A {}", None),
    (
        "export default abstract class A {}",
        Some((
            "module_illegal_default_export",
            "A component cannot have a default export",
            19,
        )),
    ),
    ("class A<T> extends B<T> implements C<T>, D {}", None),
    (
        "class A implements B extends C {}",
        Some(("js_parse_error", "Unexpected token", 40)),
    ),
    (
        "class A extends B, C {}",
        Some(("js_parse_error", "Unexpected token", 36)),
    ),
    ("interface A extends B, C {}", None),
    (
        "interface A implements B {}",
        Some(("js_parse_error", "Unexpected token", 31)),
    ),
    ("let a: A<B<C>>", None),
    ("let a: A<B<C<D>>>", None),
    ("let a = b < c > d", None),
    ("let a = f<b>(c)", None),
    ("let a = f<b>", None),
    ("let a = new A<b>", None),
    ("let a = f<b>`c`", None),
    ("let a: (b: number) => void", None),
    ("let a: new () => A", None),
    ("let a: abstract new () => A", None),
    ("let a: { [k: string]: number }", None),
    ("let a: { [k in K]: number }", None),
    ("let a: { readonly [k in K]?: number }", None),
    ("let a: { -readonly [k in K]-?: number }", None),
    ("let a: `a${string}`", None),
    ("let a: keyof typeof b", None),
    ("let a: A[number]['b']", None),
    ("let a: [a: string, b?: number, ...c: boolean[]]", None),
    ("let a: () => asserts this", None),
    ("function f<T extends U, U = T>() {}", None),
    ("function f<T = 1, U>() {}", None),
    ("function f(...a: number[]) {}", None),
    (
        "function f(...a?: number[]) {}",
        Some((
            "js_parse_error",
            "A binding pattern parameter cannot be optional in an implementation signature.",
            30,
        )),
    ),
    (
        "function f(...a = []) {}",
        Some(("js_parse_error", "Unexpected token", 35)),
    ),
    ("class A { private constructor() {} }", None),
    ("class A { static constructor() {} }", None),
    ("class A { constructor(): void {} }", None),
    ("class A { m?() {} }", None),
    (
        "class A { m!() {} }",
        Some(("js_parse_error", "Unexpected token", 31)),
    ),
    ("class A { x: number = 1; static y?: string }", None),
    ("class A { declare x: number }", None),
    ("class A { declare readonly x: number }", None),
    ("class A { abstract }", None),
    ("class A { declare }", None),
    ("class A { readonly }", None),
    ("class A { static }", None),
    ("class A { public }", None),
    ("class A { override }", None),
    ("class A { get }", None),
    ("class A { accessor }", None),
    ("let abstract", None),
    ("let declare", None),
    ("let type", None),
    ("type type = 1", None),
    ("let namespace", None),
    ("namespace namespace {}", None),
    ("let module", None),
    ("module module {}", None),
    ("let readonly", None),
    ("let as = 1 as as", None),
    ("let is", None),
    ("let asserts", None),
    ("let infer", None),
    ("let keyof", None),
    ("let unique", None),
    ("let satisfies", None),
    ("let of", None),
    ("let async", None),
    ("let override", None),
    ("let global", None),
    ("global = 1", None),
    (
        "@dec class A {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "class A { @dec m() {} }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            29,
        )),
    ),
    (
        "class A { @dec x }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            29,
        )),
    ),
    (
        "class A { m(@dec a) {} }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            31,
        )),
    ),
    (
        "@dec export class A {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "export @dec class A {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            26,
        )),
    ),
    (
        "@dec function f() {}",
        Some((
            "js_parse_error",
            "Leading decorators must be attached to a class declaration.",
            24,
        )),
    ),
    (
        "function f(@dec a) {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            30,
        )),
    ),
    (
        "let a = @dec class {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like decorators (related TSC proposal is not stage 4 yet) are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            27,
        )),
    ),
    ("type A = string", None),
    ("type A<T> = T", None),
    (
        "type A = ",
        Some(("js_parse_error", "Unexpected token", 29)),
    ),
    ("type = 1", None),
    ("let a: ", Some(("js_parse_error", "Unexpected token", 27))),
    (
        "let a: 1 +",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    ("function f(): {}", None),
    ("function f(): { }", None),
    ("let a = (b): c => d", None),
    (
        "let a = (b)!: c",
        Some(("js_parse_error", "Unexpected token", 31)),
    ),
    (
        "let a = b ? (c) : d => e",
        Some(("js_parse_error", "Unexpected token", 44)),
    ),
    (
        "let a = b ? (c): d => e",
        Some(("js_parse_error", "Unexpected token", 43)),
    ),
    ("let a = b ? (c): d => e : f", None),
    ("async <T>(a: T) => a", None),
    ("async < T > (a)", None),
    ("let a = async<T>(b)", None),
    ("class A { [key: string]: any; [k2: number]: any }", None),
    ("class A { m<T>(this: T): T { return this } }", None),
    (
        "class A { get x(): number { return 1 } set x(v: number) {} }",
        None,
    ),
    ("class A { static async *m() {} }", None),
    ("class A { static get x() { return 1 } }", None),
    ("class A { declare static x: number }", None),
    ("class A { static declare x: number }", None),
    (
        "class A { protected abstract x }",
        Some((
            "js_parse_error",
            "Abstract methods can only appear within an abstract class.",
            29,
        )),
    ),
    ("abstract class A { protected abstract x: number }", None),
    ("abstract class A { abstract protected x: number }", None),
    ("abstract class A { private abstract x: number }", None),
    (
        "abstract class A { abstract override x: number }",
        Some((
            "js_parse_error",
            "This member cannot have an 'override' modifier because its containing class does not extend another class.",
            38,
        )),
    ),
    (
        "abstract class A { override abstract x: number }",
        Some((
            "js_parse_error",
            "'abstract' modifier must precede 'override' modifier.",
            28,
        )),
    ),
    (
        "abstract class A extends B { abstract override x: number }",
        None,
    ),
    ("abstract class A { abstract async m(): void }", None),
    ("abstract class A { abstract get x(): number }", None),
    (
        "abstract class A { abstract get x() { return 1 } }",
        Some((
            "js_parse_error",
            "Method 'x' cannot have an implementation because it is marked abstract.",
            38,
        )),
    ),
    ("abstract class A { abstract constructor() }", None),
    (
        "abstract class A { abstract static {} }",
        Some((
            "js_parse_error",
            "Static class blocks cannot have any modifier.",
            55,
        )),
    ),
    (
        "class A { override constructor() {} }",
        Some((
            "js_parse_error",
            "This member cannot have an 'override' modifier because its containing class does not extend another class.",
            29,
        )),
    ),
    ("class A extends B { override constructor() {} }", None),
    (
        "class A { static override m() {} }",
        Some((
            "js_parse_error",
            "This member cannot have an 'override' modifier because its containing class does not extend another class.",
            29,
        )),
    ),
    ("export type A = 1", None),
    ("export type { A }", None),
    ("export type * from 'a'", None),
    ("export type * as b from 'a'", None),
    ("export { type A }", None),
    ("export { type A as B }", None),
    ("export { type as }", None),
    ("export { type }", None),
    ("import { type } from 'a'", None),
    ("import { type as } from 'a'", None),
    ("import { type as as } from 'a'", None),
    ("import { type as as as } from 'a'", None),
    ("import type from 'a'", None),
    ("import type, { a } from 'b'", None),
    ("import type * as a from 'b'", None),
    (
        "import type a, * as b from 'c'",
        Some((
            "js_parse_error",
            "A type-only import can specify a default import or named bindings, but not both.",
            19,
        )),
    ),
    ("export import a = b.c", None),
    (
        "export import type a = b.c",
        Some((
            "js_parse_error",
            "An import alias can not use 'import type'.",
            42,
        )),
    ),
    ("import type a = require('b')", None),
    (
        "export default interface A {}",
        Some((
            "module_illegal_default_export",
            "A component cannot have a default export",
            19,
        )),
    ),
    (
        "export default enum E {}",
        Some(("js_parse_error", "The keyword 'enum' is reserved", 34)),
    ),
    (
        "export default type A = 1",
        Some(("js_parse_error", "Unexpected token", 39)),
    ),
    (
        "export default abstract class {}",
        Some(("js_parse_error", "Unexpected token", 49)),
    ),
    ("export declare function f(): void", None),
    ("export declare class A {}", None),
    ("export declare namespace A {}", None),
    ("export declare const a: number", None),
    ("export declare const a = 1", None),
    ("export declare type A = 1", None),
    ("export declare interface A {}", None),
    ("export declare enum E {}", None),
    ("declare type A = 1", None),
    ("declare interface A {}", None),
    (
        "declare let a: number, b = 1",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            46,
        )),
    ),
    (
        "declare using a: number",
        Some(("js_parse_error", "Unexpected token", 27)),
    ),
    ("declare class A { m() {} }", None),
    ("declare class A { m(): void }", None),
    ("declare class A { constructor() {} }", None),
    ("declare class A { get x() { return 1 } }", None),
    ("declare class A { static {} }", None),
    (
        "declare namespace A { let a = 1 }",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            49,
        )),
    ),
    (
        "declare namespace A { function f() {} }",
        Some((
            "js_parse_error",
            "An implementation cannot be declared in ambient contexts.",
            41,
        )),
    ),
    (
        "declare namespace A { class B { m() {} } }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like namespaces with non-type nodes are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "declare module A { export let a: number }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like namespaces with non-type nodes are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            19,
        )),
    ),
    (
        "namespace A { declare let a = 1 }",
        Some((
            "js_parse_error",
            "Initializers are not allowed in ambient contexts.",
            49,
        )),
    ),
    (
        "namespace A { declare function f() {} }",
        Some((
            "js_parse_error",
            "An implementation cannot be declared in ambient contexts.",
            33,
        )),
    ),
    ("function f() { declare let a: number }", None),
    ("function f() { declare function g(): void }", None),
    ("function f() { namespace A {} }", None),
    (
        "function f() { enum E {} }",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            34,
        )),
    ),
    ("function f() { type A = 1 }", None),
    ("function f() { interface A {} }", None),
    ("if (a) type A = 1", None),
    ("if (a) interface A {}", None),
    (
        "if (a) enum E {}",
        Some((
            "typescript_invalid_feature",
            "TypeScript language features like enums are not natively supported, and their use is generally discouraged. Outside of `<script>` tags, these features are not supported. For use within `<script>` tags, you will need to use a preprocessor to convert it to JavaScript before it gets passed to the Svelte compiler. If you are using `vitePreprocess`, make sure to specifically enable preprocessing script tags (`vitePreprocess({ script: true })`)",
            26,
        )),
    ),
    ("if (a) namespace A {}", None),
    ("if (a) declare let b: number", None),
    ("label: type A = 1", None),
    ("label: interface A {}", None),
    (
        "abstract function f() {}",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    (
        "declare abstract function f(): void",
        Some(("js_parse_error", "Unexpected token", 36)),
    ),
    (
        "abstract const a = 1",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    (
        "abstract enum E {}",
        Some(("js_parse_error", "Unexpected token", 19)),
    ),
    (
        "abstract type A = 1",
        Some(("js_parse_error", "Unexpected token", 19)),
    ),
    (
        "abstract namespace A {}",
        Some(("js_parse_error", "Unexpected token", 19)),
    ),
    (
        "override function f() {}",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    (
        "readonly let a",
        Some(("js_parse_error", "Unexpected token", 28)),
    ),
    (
        "static function f() {}",
        Some(("js_parse_error", "The keyword 'static' is reserved", 19)),
    ),
    (
        "public class A {}",
        Some(("js_parse_error", "The keyword 'public' is reserved", 19)),
    ),
    (
        "private let a",
        Some(("js_parse_error", "The keyword 'private' is reserved", 19)),
    ),
];

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(code, expected) in CASES {
        let src = format!("<script lang=\"ts\">\n{code}\n</script>\n");
        let options = LintOptions {
            runes: Some(true),
            preprocess_configured: true,
            ..LintOptions::default()
        };
        let warnings = svn_lint::lint_file_with_options(
            &src,
            Path::new("t.svelte"),
            options,
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
            failures.push(format!("{code:?}\n  got:  {got:?}\n  want: {expected:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
