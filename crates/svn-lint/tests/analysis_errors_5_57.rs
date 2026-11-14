//! Three errors the compiler raises while it analyses a component,
//! which the lint has to report itself (the component still converts,
//! so nothing else names them).
//!
//! - `let_directive_snippet_conflict`: a `let:` directive on a component
//!   that has a `children` snippet as a direct child. The snippet
//!   replaces the default slot content, so the directive would never be
//!   given a value. A component placed in a named slot (`slot="name"`,
//!   as text) is exempt: there the directive belongs to that slot.
//! - `derived_invalid_export`: an exported declaration initialised with
//!   `$derived`, `let` included. It is checked before the rule that
//!   refuses `export let` in runes mode.
//! - `declaration_duplicate`: a name declared twice in one template
//!   scope. The parameters of a snippet, the item and index of an
//!   `{#each}` and a `then` / `catch` value share their scope with the
//!   tags written directly in the block; an `{#if}` inside starts a new
//!   one.
//!
//! Each case is what svelte 5.57.2's `compile()` throws: code, first
//! line of the message, and the byte range it points at.

use std::path::Path;
use svn_lint::CompatFeatures;

type Expected = Option<(&'static str, &'static str, u32, u32)>;

const CASES: &[(&str, Expected)] = &[
    (
        "<script>import C from './C.svelte';</script>\n<C let:x>{#snippet children()}a{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x })}`)",
            48,
            53,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x={y}>{#snippet children()}a{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x: y })}`)",
            48,
            57,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x={{ a, b }}>{#snippet children()}a{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x: { a, b } })}`)",
            48,
            64,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x={x}>{#snippet children()}a{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x })}`)",
            48,
            57,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x slot=\"s\">{#snippet children()}a{/snippet}</C>",
        None,
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x slot={s}>{#snippet children()}a{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x })}`)",
            48,
            53,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x>{#snippet other()}a{/snippet}</C>",
        None,
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x>{#if a}{#snippet children()}a{/snippet}{/if}</C>",
        None,
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x let:y>{#snippet children()}a{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x })}`)",
            48,
            53,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<svelte:component this={C} let:x>{#snippet children()}a{/snippet}</svelte:component>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ x })}`)",
            72,
            77,
        )),
    ),
    ("<div let:x>{#snippet children()}a{/snippet}</div>", None),
    (
        "<script>import C from './C.svelte';</script>\n<C let:x>a</C>",
        None,
    ),
    (
        "<script>export let x = $derived(1);</script>",
        Some((
            "derived_invalid_export",
            "Cannot export derived state from a module. To expose the current derived value, export a function returning its value",
            8,
            35,
        )),
    ),
    (
        "<script>export const x = $derived(1);</script>",
        Some((
            "derived_invalid_export",
            "Cannot export derived state from a module. To expose the current derived value, export a function returning its value",
            8,
            37,
        )),
    ),
    (
        "<script>let a = $state(0); export let x = $derived(a);</script>",
        Some((
            "derived_invalid_export",
            "Cannot export derived state from a module. To expose the current derived value, export a function returning its value",
            27,
            54,
        )),
    ),
    (
        "<script>export let x = $state(0); x = 1;</script>",
        Some((
            "state_invalid_export",
            "Cannot export state from a module if it is reassigned. Either export a function returning the state value or only mutate the state value's properties",
            8,
            33,
        )),
    ),
    ("<script>export let x = 1;</script>", None),
    (
        "<script>let y = $state(0); export let x = 1;</script>",
        Some((
            "legacy_export_invalid",
            "Cannot use `export let` in runes mode — use `$props()` instead",
            27,
            44,
        )),
    ),
    (
        "<script>let { a } = $props(); export let x = $derived(a), z = 2;</script>",
        Some((
            "derived_invalid_export",
            "Cannot export derived state from a module. To expose the current derived value, export a function returning its value",
            30,
            64,
        )),
    ),
    (
        "{#snippet row2(item)}{const item = 1}<p>{item}</p>{/snippet}",
        Some((
            "declaration_duplicate",
            "`item` has already been declared",
            28,
            32,
        )),
    ),
    (
        "{#snippet row1(item)}{#if item}{const item = 1}<p>{item}</p>{/if}{/snippet}",
        None,
    ),
    (
        "{#snippet row3(item)}{let item = 1}{item}{/snippet}",
        Some((
            "declaration_duplicate",
            "`item` has already been declared",
            26,
            30,
        )),
    ),
    (
        "{#snippet row4({ item })}{const item = 1}{item}{/snippet}",
        Some((
            "declaration_duplicate",
            "`item` has already been declared",
            32,
            36,
        )),
    ),
    (
        "{#snippet row5(a)}{const b = 1}{const b = 2}{b}{/snippet}",
        Some((
            "declaration_duplicate",
            "`b` has already been declared",
            38,
            39,
        )),
    ),
    (
        "{const b = 1}{const b = 2}{b}",
        Some((
            "declaration_duplicate",
            "`b` has already been declared",
            20,
            21,
        )),
    ),
    (
        "{#each [1] as item}{const item = 1}{item}{/each}",
        Some((
            "declaration_duplicate",
            "`item` has already been declared",
            26,
            30,
        )),
    ),
    (
        "{#each [1] as item, i}{const i = 1}{i}{/each}",
        Some((
            "declaration_duplicate",
            "`i` has already been declared",
            29,
            30,
        )),
    ),
    (
        "{#await p then v}{const v = 1}{v}{/await}",
        Some((
            "declaration_duplicate",
            "`v` has already been declared",
            15,
            16,
        )),
    ),
    (
        "{#if x}{const b = 1}{const b = 2}{b}{/if}",
        Some((
            "declaration_duplicate",
            "`b` has already been declared",
            27,
            28,
        )),
    ),
    (
        "{#snippet row6(item)}{@const item = 1}{item}{/snippet}",
        Some((
            "declaration_duplicate",
            "`item` has already been declared",
            29,
            33,
        )),
    ),
    (
        "{#each [1] as item}{@const item = 1}{item}{/each}",
        Some((
            "declaration_duplicate",
            "`item` has already been declared",
            27,
            31,
        )),
    ),
    (
        "<div>{const b = 1}{const b = 2}{b}</div>",
        Some((
            "declaration_duplicate",
            "`b` has already been declared",
            25,
            26,
        )),
    ),
    (
        "{#snippet a()}x{/snippet}{#snippet a()}y{/snippet}",
        Some((
            "declaration_duplicate",
            "`a` has already been declared",
            35,
            36,
        )),
    ),
    (
        "{const a = 1}{#snippet a()}x{/snippet}",
        Some((
            "declaration_duplicate",
            "`a` has already been declared",
            23,
            24,
        )),
    ),
    (
        "{#snippet a()}x{/snippet}{const a = 1}",
        Some((
            "declaration_duplicate",
            "`a` has already been declared",
            32,
            33,
        )),
    ),
    // Parentheses around the whole expression and comments beside it
    // are not part of it.
    (
        "<script>import C from './C.svelte';</script>\n<C let:item={(item)}>{#snippet children()}x{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ item })}`)",
            48,
            65,
        )),
    ),
    (
        "<script>import C from './C.svelte';</script>\n<C let:item={({a})}>{#snippet children()}x{/snippet}</C>",
        Some((
            "let_directive_snippet_conflict",
            "Cannot use `let:` directives on a component that has a `children` snippet. Use snippet parameters instead (e.g. `{#snippet children({ item: {a} })}`)",
            48,
            64,
        )),
    ),
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
