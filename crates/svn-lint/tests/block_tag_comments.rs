//! The compiler reads the parts of a block tag that sit outside any
//! JavaScript expression (the `as` pattern of an `{#each}`, its index
//! and key, the value of a `then` / `catch`, the name a `{@const}`
//! declares, the whitespace after a tag keyword) with its own
//! character-level reader, which knows nothing about JavaScript
//! comments. A comment that acorn reads as part of an expression is
//! fine; one written where that reader looks for the next token is an
//! error, reported where the comment starts. Each case below is what
//! svelte 5.57.2's `parse()` reports for the template, with the
//! component in JavaScript and in TypeScript. A type annotation on a
//! pattern is read the same way, and fails without TypeScript.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32)>;

/// `(typescript, template, the compiler's error: code, message, offset)`.
/// Offsets count from the start of the file, after the script line.
const CASES: &[(bool, &str, Expected)] = &[
    (
        false,
        "{#each [] as q /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        false,
        "{#each [] as q//c\n}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            44,
        )),
    ),
    (
        false,
        "{#each [] as /*c*/ q}{/each}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            43,
        )),
    ),
    (
        false,
        "{#each [] as q/*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            44,
        )),
    ),
    (
        false,
        "{#each [] as {a} /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        false,
        "{#each [] as [a, b] /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            50,
        )),
    ),
    (
        false,
        "{#each [] as q /*c*/, i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        false,
        "{#each [] as q, /*c*/ i}{/each}",
        Some((
            "expected_identifier",
            "Expected an identifier\nhttps://svelte.dev/e/expected_identifier",
            46,
        )),
    ),
    (
        false,
        "{#each [] as q, i /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        false,
        "{#each [] as q, i /*c*/ (q)}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        false,
        "{#each [] as q (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            49,
        )),
    ),
    (
        false,
        "{#each [] as q, i (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            52,
        )),
    ),
    (
        false,
        "{#each [] as q, i (q.a) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            54,
        )),
    ),
    (
        false,
        "{#each [] as q, i (q)\n//c\n}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            52,
        )),
    ),
    (
        false,
        "{#each [] as {a}, i (a) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            54,
        )),
    ),
    (
        false,
        "{#each [] as {a = 1}, i (a) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            58,
        )),
    ),
    (
        false,
        "{#each [] as q, i (q /*c*/) /*d*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            58,
        )),
    ),
    (
        false,
        "{#each [] as q, i (/*c*/q) /*d*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            57,
        )),
    ),
    (
        false,
        "{#each [] /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            40,
        )),
    ),
    (
        false,
        "{#each []/*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            39,
        )),
    ),
    (
        false,
        "{#each [] /*c*/, i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            40,
        )),
    ),
    (
        false,
        "{#each [], i /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        false,
        "{#each [], i (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            52,
        )),
    ),
    (
        false,
        "{#each f() /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        false,
        "{#await p then v /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        false,
        "{#await p catch e /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        false,
        "{#await p then {a} /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            49,
        )),
    ),
    (
        false,
        "{#await p}{:then /*c*/ v}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            47,
        )),
    ),
    (
        false,
        "{#await p}{:then v /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            49,
        )),
    ),
    (
        false,
        "{#await p}{:then /*c*/}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            47,
        )),
    ),
    (
        false,
        "{#await p}{:then/*c*/ v}{/await}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            46,
        )),
    ),
    (
        false,
        "{#await p}{:catch /*c*/ e}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            48,
        )),
    ),
    (
        false,
        "{#await p}{:catch e /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            50,
        )),
    ),
    (
        false,
        "{#await p}{:then v}{:catch /*c*/}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            57,
        )),
    ),
    (
        false,
        "{#if 1}{@const /*c*/ a = 1}{/if}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            45,
        )),
    ),
    (
        false,
        "{#if 1}{@const a /*c*/ = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        false,
        "{#if 1}{@const {a} /*c*/ = b}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            49,
        )),
    ),
    (
        false,
        "{#if 1}{@const a\n/*c*/ = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        false,
        "{#if 1}{@const/*c*/ a = 1}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            44,
        )),
    ),
    (
        false,
        "{#each/*c*/ [] as q}{/each}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        false,
        "{#if/*c*/ a}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        false,
        "{#await/*c*/ p}{/await}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            37,
        )),
    ),
    (
        false,
        "{#key/*c*/ a}{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        false,
        "{ #if/*c*/ a}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        false,
        "{@html/*c*/ a}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        false,
        "{@render/*c*/ a()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            38,
        )),
    ),
    (
        false,
        "{#if a}{:else if/*c*/ a}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            46,
        )),
    ),
    (false, "{#each [] /*c*/ as q}{/each}", None),
    (false, "{#each /*c*/ [] as q}{/each}", None),
    (false, "{#each [] as {a /*c*/}}{/each}", None),
    (false, "{#each [] as q, i (/*c*/ q)}{/each}", None),
    (false, "{#each [] as q, i (q /*c*/)}{/each}", None),
    (false, "{#await p /*c*/ then v}{/await}", None),
    (false, "{#await /*c*/ p}{/await}", None),
    (false, "{#snippet name(/*c*/ a)}{/snippet}", None),
    (false, "{#if 1}{@const a = /*c*/ 1}{/if}", None),
    (false, "{#if 1}{@const a = 1 /*c*/}{/if}", None),
    (false, "{#key a /*c*/}{/key}", None),
    (false, "{#if /*c*/ a /*c*/}{/if}", None),
    (
        false,
        "{#await p}{:then }{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            47,
        )),
    ),
    (
        false,
        "{#await p}{:catch\n}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            48,
        )),
    ),
    (
        false,
        "{#await p then v}{:catch }{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            55,
        )),
    ),
    (false, "{#await p then }{/await}", None),
    (false, "{#await p catch }{/await}", None),
    (false, "{#await p}{:then}{:catch}{/await}", None),
    (false, "{@html a /*c*/}", None),
    (false, "{@debug/*c*/ a}", None),
    (false, "{#each [] as  q , i  ( q )  }{/each}", None),
    (false, "{#if a}{:else if a /*c*/}{/if}", None),
    (
        false,
        "{#each [] as q: string}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        false,
        "{#each [] as {a, b}: T}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        false,
        "{#each [] as q: string /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        false,
        "{#each [] as {a}: T /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        false,
        "{#each [] as q:T, i (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        false,
        "{#await p}{:then {a}: T /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        false,
        "{#if 1}{@const a: T /*c*/ = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        false,
        "{#each [] as q: /*c*/ string}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        true,
        "{#each [] as q /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        true,
        "{#each [] as q//c\n}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            42,
        )),
    ),
    (
        true,
        "{#each [] as /*c*/ q}{/each}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            41,
        )),
    ),
    (
        true,
        "{#each [] as q/*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            42,
        )),
    ),
    (
        true,
        "{#each [] as {a} /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#each [] as [a, b] /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        true,
        "{#each [] as q /*c*/, i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            43,
        )),
    ),
    (
        true,
        "{#each [] as q, /*c*/ i}{/each}",
        Some((
            "expected_identifier",
            "Expected an identifier\nhttps://svelte.dev/e/expected_identifier",
            44,
        )),
    ),
    (
        true,
        "{#each [] as q, i /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            46,
        )),
    ),
    (
        true,
        "{#each [] as q, i /*c*/ (q)}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            46,
        )),
    ),
    (
        true,
        "{#each [] as q (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#each [] as q, i (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            55,
        )),
    ),
    (
        true,
        "{#each [] as q, i (q.a) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            57,
        )),
    ),
    (
        true,
        "{#each [] as q, i (q)\n//c\n}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            54,
        )),
    ),
    (
        true,
        "{#each [] as {a}, i (a) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            57,
        )),
    ),
    (
        true,
        "{#each [] as {a = 1}, i (a) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            56,
        )),
    ),
    (
        true,
        "{#each [] as q, i (q /*c*/) /*d*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            56,
        )),
    ),
    (
        true,
        "{#each [] as q, i (/*c*/q) /*d*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            55,
        )),
    ),
    (
        true,
        "{#each [] /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (
        true,
        "{#each []/*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            37,
        )),
    ),
    (
        true,
        "{#each [] /*c*/, i}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            38,
        )),
    ),
    (
        true,
        "{#each [], i /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            41,
        )),
    ),
    (
        true,
        "{#each [], i (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            50,
        )),
    ),
    (
        true,
        "{#each f() /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            39,
        )),
    ),
    (
        true,
        "{#await p then v /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#await p catch e /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            46,
        )),
    ),
    (
        true,
        "{#await p then {a} /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#await p}{:then /*c*/ v}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            45,
        )),
    ),
    (
        true,
        "{#await p}{:then v /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#await p}{:then /*c*/}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            45,
        )),
    ),
    (
        true,
        "{#await p}{:then/*c*/ v}{/await}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            44,
        )),
    ),
    (
        true,
        "{#await p}{:catch /*c*/ e}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            46,
        )),
    ),
    (
        true,
        "{#await p}{:catch e /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        true,
        "{#await p}{:then v}{:catch /*c*/}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            55,
        )),
    ),
    (
        true,
        "{#if 1}{@const /*c*/ a = 1}{/if}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            43,
        )),
    ),
    (
        true,
        "{#if 1}{@const a /*c*/ = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#if 1}{@const {a} /*c*/ = b}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            47,
        )),
    ),
    (
        true,
        "{#if 1}{@const a\n/*c*/ = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            45,
        )),
    ),
    (
        true,
        "{#if 1}{@const/*c*/ a = 1}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            42,
        )),
    ),
    (
        true,
        "{#each/*c*/ [] as q}{/each}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        true,
        "{#if/*c*/ a}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            32,
        )),
    ),
    (
        true,
        "{#await/*c*/ p}{/await}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            35,
        )),
    ),
    (
        true,
        "{#key/*c*/ a}{/key}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            33,
        )),
    ),
    (
        true,
        "{ #if/*c*/ a}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            33,
        )),
    ),
    (
        true,
        "{@html/*c*/ a}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            34,
        )),
    ),
    (
        true,
        "{@render/*c*/ a()}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            36,
        )),
    ),
    (
        true,
        "{#if a}{:else if/*c*/ a}{/if}",
        Some((
            "expected_whitespace",
            "Expected whitespace\nhttps://svelte.dev/e/expected_whitespace",
            44,
        )),
    ),
    (true, "{#each [] /*c*/ as q}{/each}", None),
    (true, "{#each /*c*/ [] as q}{/each}", None),
    (true, "{#each [] as {a /*c*/}}{/each}", None),
    (true, "{#each [] as q, i (/*c*/ q)}{/each}", None),
    (true, "{#each [] as q, i (q /*c*/)}{/each}", None),
    (true, "{#await p /*c*/ then v}{/await}", None),
    (true, "{#await /*c*/ p}{/await}", None),
    (true, "{#snippet name(/*c*/ a)}{/snippet}", None),
    (true, "{#if 1}{@const a = /*c*/ 1}{/if}", None),
    (true, "{#if 1}{@const a = 1 /*c*/}{/if}", None),
    (true, "{#key a /*c*/}{/key}", None),
    (true, "{#if /*c*/ a /*c*/}{/if}", None),
    (
        true,
        "{#await p}{:then }{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            45,
        )),
    ),
    (
        true,
        "{#await p}{:catch\n}{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            46,
        )),
    ),
    (
        true,
        "{#await p then v}{:catch }{/await}",
        Some((
            "expected_pattern",
            "Expected identifier or destructure pattern\nhttps://svelte.dev/e/expected_pattern",
            53,
        )),
    ),
    (true, "{#await p then }{/await}", None),
    (true, "{#await p catch }{/await}", None),
    (true, "{#await p}{:then}{:catch}{/await}", None),
    (true, "{@html a /*c*/}", None),
    (true, "{@debug/*c*/ a}", None),
    (true, "{#each [] as  q , i  ( q )  }{/each}", None),
    (true, "{#if a}{:else if a /*c*/}{/if}", None),
    (true, "{#each [] as q: string}{/each}", None),
    (true, "{#each [] as {a, b}: T}{/each}", None),
    (
        true,
        "{#each [] as q: string /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            51,
        )),
    ),
    (
        true,
        "{#each [] as {a}: T /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (
        true,
        "{#each [] as q:T, i (q) /*c*/}{/each}",
        Some((
            "expected_token",
            "Expected token )\nhttps://svelte.dev/e/expected_token",
            57,
        )),
    ),
    (
        true,
        "{#await p}{:then {a}: T /*c*/}{/await}",
        Some((
            "expected_token",
            "Expected token }\nhttps://svelte.dev/e/expected_token",
            52,
        )),
    ),
    (
        true,
        "{#if 1}{@const a: T /*c*/ = 1}{/if}",
        Some((
            "expected_token",
            "Expected token =\nhttps://svelte.dev/e/expected_token",
            48,
        )),
    ),
    (true, "{#each [] as q: /*c*/ string}{/each}", None),
];

fn source(ts: bool, template: &str) -> String {
    let script = if ts {
        "<script lang=\"ts\"></script>\n"
    } else {
        "<script>let p, a, b;</script>\n"
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
        let got = warnings.iter().find(|w| w.is_error).map(|w| {
            (
                w.code.as_str(),
                w.message.as_str(),
                w.range.start,
                w.range.end,
            )
        });
        let want = expected.map(|(code, message, at)| (code, message, at, at));
        if got != want {
            failures.push(format!(
                "ts={ts} {template:?}\n  got:  {got:?}\n  want: {want:?}"
            ));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// A component whose template the compiler rejects leaves the run, as
/// svelte2tsx (which calls `parse()`) leaves it out.
#[test]
fn rejected_templates_are_detected() {
    let mut failures = Vec::new();
    for &(ts, template, expected) in CASES {
        let src = source(ts, template);
        let (doc, _) = svn_parser::parse_sections(&src);
        let (fragment, errors) = svn_parser::parse_all_template_runs(&src, &doc.template.text_runs);
        let rejected = errors.iter().any(|e| e.compiler_rejects())
            || svn_lint::template_parse_rejected(&fragment, &src);
        if rejected != expected.is_some() {
            failures.push(format!("ts={ts} {template:?}: rejected={rejected}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
