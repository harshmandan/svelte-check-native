//! How acorn reads a destructuring pattern (`{#each … as`, `then`,
//! `catch`, `{@const}`) and a snippet's parameters.
//!
//! The compiler wraps a pattern as `(pattern = 1)` and a parameter list
//! as an arrow function's, and has acorn read that. acorn has its own
//! messages for a keyword used as a shorthand name, a reserved word, and
//! a comma after a rest element, and it throws each where the word or
//! comma is. Each case below is what svelte 5.57.2's `parse()` reports
//! for the template.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, script line included.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{#each cases as { case }}\n\t{case.title}\n{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'case'\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        true,
        "{#each cases as { case }}\n\t{case.title}\n{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'case'\nhttps://svelte.dev/e/js_parse_error",
            46,
        )),
    ),
    (
        false,
        "{#each cases as { case }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'case'\nhttps://svelte.dev/e/js_parse_error",
            36,
        )),
    ),
    (
        true,
        "{#each cases as { case }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'case'\nhttps://svelte.dev/e/js_parse_error",
            46,
        )),
    ),
    (
        false,
        "{#snippet s({ class })}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#snippet s({ class })}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#snippet s(class)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        true,
        "{#snippet s(class)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            45,
        )),
    ),
    (
        false,
        "{#snippet s(a, { b, class })}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            38,
        )),
    ),
    (
        true,
        "{#snippet s(a, { b, class })}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            48,
        )),
    ),
    (
        false,
        "{#each a as { animal, features: { ...rest, eyes } } }x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            59,
        )),
    ),
    (
        true,
        "{#each a as { animal, features: { ...rest, eyes } } }x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            69,
        )),
    ),
    (
        false,
        "{#each a as [ class ]}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            38,
        )),
    ),
    (
        true,
        "{#each a as [ class ]}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            48,
        )),
    ),
    (
        false,
        "{#each a as { a: class }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            41,
        )),
    ),
    (
        true,
        "{#each a as { a: class }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            51,
        )),
    ),
    (
        false,
        "{#each a as { class = 1 }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { class = 1 }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { x, class }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        true,
        "{#each a as { x, class }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            45,
        )),
    ),
    (
        false,
        "{#await p then { class }}x{/await}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        true,
        "{#await p then { class }}x{/await}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            45,
        )),
    ),
    (
        false,
        "{#await p}{:then { class }}x{/await}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        true,
        "{#await p}{:then { class }}x{/await}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (
        false,
        "{#await p}{:catch { case }}x{/await}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'case'\nhttps://svelte.dev/e/js_parse_error",
            38,
        )),
    ),
    (
        true,
        "{#await p}{:catch { case }}x{/await}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'case'\nhttps://svelte.dev/e/js_parse_error",
            48,
        )),
    ),
    (
        false,
        "{#if x}{@const { class } = a}{/if}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        true,
        "{#if x}{@const { class } = a}{/if}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            45,
        )),
    ),
    (
        false,
        "{#each a as { ...rest, eyes }}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        true,
        "{#each a as { ...rest, eyes }}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            49,
        )),
    ),
    (
        false,
        "{#each a as [...rest, eyes]}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            38,
        )),
    ),
    (
        true,
        "{#each a as [...rest, eyes]}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            48,
        )),
    ),
    (
        false,
        "{#snippet s({ ...rest, eyes })}x{/snippet}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        true,
        "{#snippet s({ ...rest, eyes })}x{/snippet}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            49,
        )),
    ),
    (
        false,
        "{#snippet s(...rest, eyes)}x{/snippet}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        true,
        "{#snippet s(...rest, eyes)}x{/snippet}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (
        false,
        "{#each a as { yield }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { yield }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { await }}x{/each}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { await }}x{/each}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { let }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'let' is reserved\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { let }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'let' is reserved\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { enum }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'enum' is reserved\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { enum }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'enum' is reserved\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { a b }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        true,
        "{#each a as { a b }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            44,
        )),
    ),
    (
        false,
        "{#each a as { 1 }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        true,
        "{#each a as { 1 }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            44,
        )),
    ),
    (
        false,
        "{#snippet s({ a b })}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            34,
        )),
    ),
    (
        true,
        "{#snippet s({ a b })}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            44,
        )),
    ),
    (
        false,
        "{#snippet s(a = class)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        true,
        "{#snippet s(a = class)}x{/snippet}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            49,
        )),
    ),
    (
        false,
        "{#each a as { class }, i}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { class }, i}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'class'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { eval }}x{/each}",
        Some((
            "js_parse_error",
            "Assigning to eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { eval }}x{/each}",
        Some((
            "js_parse_error",
            "Assigning to eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { arguments }}x{/each}",
        Some((
            "js_parse_error",
            "Assigning to arguments in strict mode\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { arguments }}x{/each}",
        Some((
            "js_parse_error",
            "Assigning to arguments in strict mode\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { static }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'static' is reserved\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { static }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'static' is reserved\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { implements }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { implements }}x{/each}",
        Some((
            "js_parse_error",
            "The keyword 'implements' is reserved\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { this }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'this'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { this }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'this'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { null }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'null'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { null }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'null'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { true }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'true'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { true }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'true'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { new }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'new'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { new }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'new'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { typeof }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'typeof'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { typeof }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'typeof'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { function }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'function'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { function }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'function'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { in }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'in'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { in }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'in'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (false, "{#each a as { async }}x{/each}", None),
    (true, "{#each a as { async }}x{/each}", None),
    (false, "{#each a as { of }}x{/each}", None),
    (true, "{#each a as { of }}x{/each}", None),
    (
        false,
        "{#each a as [...rest,]}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            38,
        )),
    ),
    (
        true,
        "{#each a as [...rest,]}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            48,
        )),
    ),
    (
        false,
        "{#each a as { ...rest, }}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            39,
        )),
    ),
    (
        true,
        "{#each a as { ...rest, }}x{/each}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            49,
        )),
    ),
    (
        false,
        "{#each a as { ...{ b } }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            35,
        )),
    ),
    (
        true,
        "{#each a as { ...{ b } }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected token\nhttps://svelte.dev/e/js_parse_error",
            45,
        )),
    ),
    (
        false,
        "{#snippet s({ yield })}x{/snippet}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#snippet s({ yield })}x{/snippet}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#snippet s({ await })}x{/snippet}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#snippet s({ await })}x{/snippet}",
        Some((
            "js_parse_error",
            "Cannot use keyword 'await' outside an async function\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#snippet s({ eval })}x{/snippet}",
        Some((
            "js_parse_error",
            "Binding eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#snippet s({ eval })}x{/snippet}",
        Some((
            "js_parse_error",
            "Binding eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#snippet s(eval)}x{/snippet}",
        Some((
            "js_parse_error",
            "Binding eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        true,
        "{#snippet s(eval)}x{/snippet}",
        Some((
            "js_parse_error",
            "Binding eval in strict mode\nhttps://svelte.dev/e/js_parse_error",
            40,
        )),
    ),
    (
        false,
        "{#snippet s(yield)}x{/snippet}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            30,
        )),
    ),
    (
        true,
        "{#snippet s(yield)}x{/snippet}",
        Some((
            "js_parse_error",
            "The keyword 'yield' is reserved\nhttps://svelte.dev/e/js_parse_error",
            40,
        )),
    ),
    (
        false,
        "{#snippet s(a, a)}x{/snippet}",
        Some((
            "js_parse_error",
            "Argument name clash\nhttps://svelte.dev/e/js_parse_error",
            33,
        )),
    ),
    (
        true,
        "{#snippet s(a, a)}x{/snippet}",
        Some((
            "js_parse_error",
            "Argument name clash\nhttps://svelte.dev/e/js_parse_error",
            43,
        )),
    ),
    (
        false,
        "{#snippet s(...rest,)}x{/snippet}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            37,
        )),
    ),
    (
        true,
        "{#snippet s(...rest,)}x{/snippet}",
        Some((
            "js_parse_error",
            "Comma is not permitted after the rest element\nhttps://svelte.dev/e/js_parse_error",
            47,
        )),
    ),
    (false, "{#each a as { class: b }}x{/each}", None),
    (true, "{#each a as { class: b }}x{/each}", None),
    (false, "{#each a as { b = class {} }}x{/each}", None),
    (true, "{#each a as { b = class {} }}x{/each}", None),
    (false, "{#each a as { a, a }}x{/each}", None),
    (true, "{#each a as { a, a }}x{/each}", None),
    (
        false,
        "{#each a as { super }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'super'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { super }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'super'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { import }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'import'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { import }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'import'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
    (
        false,
        "{#each a as { export }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'export'\nhttps://svelte.dev/e/js_parse_error",
            32,
        )),
    ),
    (
        true,
        "{#each a as { export }}x{/each}",
        Some((
            "js_parse_error",
            "Unexpected keyword 'export'\nhttps://svelte.dev/e/js_parse_error",
            42,
        )),
    ),
];

fn source(ts: bool, template: &str) -> String {
    let script = if ts {
        "<script lang=\"ts\"></script>\n"
    } else {
        "<script></script>\n"
    };
    format!("{script}{template}")
}

#[test]
fn lint_reports_the_compiler_error() {
    let mut failures = Vec::new();
    for &(ts, template, expected) in CASES {
        let src = source(ts, template);
        let warnings = svn_lint::lint_file(
            &src,
            Path::new("t.svelte"),
            Some(true),
            CompatFeatures::MODERN,
        );
        let got = warnings
            .iter()
            .find(|w| w.is_error)
            .map(|w| (w.code.as_str(), w.message.as_str(), w.range.start));
        if got != expected {
            failures.push(format!(
                "ts={ts} {template:?}\n  got:  {got:?}\n  want: {expected:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The check skips type-checking a component the compiler rejects, so
/// each rejected template must be detected as one.
#[test]
fn rejected_templates_are_detected() {
    let mut failures = Vec::new();
    for &(ts, template, expected) in CASES {
        let src = source(ts, template);
        let (doc, _) = svn_parser::parse_sections(&src);
        let (fragment, errors) = svn_parser::parse_all_template_runs(&src, &doc.template.text_runs);
        let rejected = errors.iter().any(|e| e.compiler_rejects())
            || svn_lint::template_parse_rejected(&fragment, &src, doc.script_lang());
        if rejected != expected.is_some() {
            failures.push(format!("ts={ts} {template:?}: rejected={rejected}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
