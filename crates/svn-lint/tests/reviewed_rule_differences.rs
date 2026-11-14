//! Components where the lint once answered differently from the
//! compiler.
//!
//! Each case lists everything svelte 5.57.2's `compile()` reports for
//! the component: the one error it throws, or its warnings. An entry is
//! the code, the byte range it points at, and whether it is an error.

use std::path::Path;
use svn_lint::CompatFeatures;

type Reported = (&'static str, u32, u32, bool);

const CASES: &[(&str, &[Reported])] = &[
    // A top-level snippet or declaration tag may share its name with an import.
    (
        "<script>import foo from \"./x.js\"</script>{#snippet foo()}x{/snippet}",
        &[],
    ),
    (
        "<script>import foo from \"./x.js\"</script>{let foo = 1}{foo}",
        &[],
    ),
    (
        "<script>import { foo } from \"./x.js\"</script>{#snippet foo()}x{/snippet}",
        &[],
    ),
    (
        "<script>let foo = 1;</script>{#snippet foo()}x{/snippet}",
        &[("declaration_duplicate", 39, 42, true)],
    ),
    (
        "<script>let foo = 1;</script>{let foo = 2}{foo}",
        &[("declaration_duplicate", 34, 37, true)],
    ),
    // A snippet is checked for a duplicate name after its tag, its parameters and its body.
    (
        "<script>let foo = 1;</script>{#snippet foo(...rest)}x{/snippet}",
        &[("snippet_invalid_rest_parameter", 43, 50, true)],
    ),
    (
        "<script>let foo = 1</script>{#snippet foo()}<input bind:value={1} />{/snippet}",
        &[("bind_invalid_expression", 51, 65, true)],
    ),
    (
        "<script>let foo = $state(1)</script>{ #snippet foo()}x{/snippet}",
        &[("block_unexpected_character", 36, 41, true)],
    ),
    // A typed name in a declaration tag is reported with its annotation.
    (
        "<script lang=\"ts\">let foo = 1;</script>{let a = 1, foo: number = 2}{foo}{a}",
        &[("declaration_duplicate", 51, 62, true)],
    ),
    (
        "<script lang=\"ts\">let foo = 1;</script>{let foo = 2}{foo}",
        &[("declaration_duplicate", 44, 47, true)],
    ),
    (
        "<script lang=\"ts\">let foo = 1;</script>{const { foo }: { foo: number } = { foo: 2 }}{foo}",
        &[("declaration_duplicate", 48, 51, true)],
    ),
    // An event attribute naming itself on a svelte special element.
    (
        "<svelte:window onresize={onresize} />",
        &[("attribute_global_event_reference", 15, 34, false)],
    ),
    (
        "<svelte:body onclick={onclick} />",
        &[("attribute_global_event_reference", 13, 30, false)],
    ),
    (
        "<svelte:document onkeydown={onkeydown} />",
        &[("attribute_global_event_reference", 17, 38, false)],
    ),
    (
        "<svelte:window {onresize} />",
        &[("attribute_global_event_reference", 15, 25, false)],
    ),
    (
        "<svelte:window onresize=\"{onresize}\" />",
        &[("attribute_global_event_reference", 15, 36, false)],
    ),
    (
        "<script>let onresize = () => {};</script><svelte:window onresize={onresize} />",
        &[],
    ),
    // A declaration tag in a component that only might be legacy.
    (
        "<script>let { a } = { a: 1 }; export { a };</script>{let b = 1}{b}",
        &[("declaration_tag_no_legacy_mode", 52, 63, true)],
    ),
    (
        "<script module>let m = 1;</script><script>export { m };</script>{let b = 1}{b}",
        &[("declaration_tag_no_legacy_mode", 64, 75, true)],
    ),
    ("<!-- $$props -->\n{let a = 1}\n{a}", &[]),
    ("<p>$$props</p>{let a = 1}{a}", &[]),
    (
        "<script>const s = \"$$restProps\";</script>{let a = 1}{a}{s}",
        &[],
    ),
    (
        "<script lang=\"ts\">export declare let x: number;</script>{let a = 1}{a}",
        &[],
    ),
    (
        "<script>export let x = 1;</script>{let a = 1}{a}{x}",
        &[("declaration_tag_no_legacy_mode", 34, 45, true)],
    ),
    (
        "<script>const c = 1; export { c };</script>{let a = 1}{a}",
        &[],
    ),
    (
        "<script>console.log($$props);</script>{let a = 1}{a}",
        &[("declaration_tag_no_legacy_mode", 38, 49, true)],
    ),
    (
        "{let a = 1}{a}{$$restProps.x}",
        &[("declaration_tag_no_legacy_mode", 0, 11, true)],
    ),
    (
        "<script>let x = 1; $: y = x;</script>{let a = 1}{a}{y}",
        &[("declaration_tag_no_legacy_mode", 37, 48, true)],
    ),
    // An exported declaration that TypeScript only declares is dropped whole.
    (
        "<script lang=\"ts\">export declare namespace N { enum E { A } }</script>",
        &[],
    ),
    (
        "<script lang=\"ts\">export declare namespace N { const x: number }</script>",
        &[],
    ),
    (
        "<script lang=\"ts\">export declare namespace A.B { enum E { A } }</script>",
        &[],
    ),
    (
        "<script lang=\"ts\">export declare enum E { A }</script>",
        &[],
    ),
    (
        "<script lang=\"ts\">export declare class C { constructor(private x: number) }</script>",
        &[],
    ),
    (
        "<script lang=\"ts\">export namespace N { export const x = 1 }</script>",
        &[("typescript_invalid_feature", 25, 59, true)],
    ),
    (
        "<script lang=\"ts\">declare namespace N { enum E { A } }</script>",
        &[("typescript_invalid_feature", 40, 52, true)],
    ),
    // A character reference in an attribute value or in text is read as the
    // character it stands for.
    (
        "<a href=\"/x\">&nbsp;</a>",
        &[("a11y_consider_explicit_label", 0, 23, false)],
    ),
    ("<h1>&#32;</h1>", &[("a11y_missing_content", 0, 14, false)]),
    (
        "<a href=\"&#35;\">x</a>",
        &[("a11y_invalid_attribute", 3, 15, false)],
    ),
    (
        "<a href=\"javascript&colon;void(0)\">x</a>",
        &[("a11y_invalid_attribute", 3, 34, false)],
    ),
    ("<div aria-hidden=\"&#116;rue\">x</div>", &[]),
    (
        "<img src=\"a\" alt=\"&#105;mage of x\">",
        &[("a11y_img_redundant_alt", 0, 35, false)],
    ),
    (
        "<input type=\"&#105;mage\">",
        &[("a11y_missing_attribute", 0, 25, false)],
    ),
    (
        "<div tabindex=\"&#49;\">x</div>",
        &[
            ("a11y_positive_tabindex", 5, 21, false),
            ("a11y_no_noninteractive_tabindex", 0, 29, false),
        ],
    ),
    ("<table>&#32;<tbody></tbody></table>", &[]),
    (
        "<table>&#120;<tbody></tbody></table>",
        &[("node_invalid_placement", 7, 13, true)],
    ),
    ("<a href=\"/x\">&amp;</a>", &[]),
    ("<h1>&nbsp;x</h1>", &[]),
    (
        "<div role=\"b&#117;tton\" tabindex=\"0\" onclick={() => {}} onkeydown={() => {}}>x</div>",
        &[],
    ),
    (
        "<a href=\"&num;\">x</a>",
        &[("a11y_invalid_attribute", 3, 15, false)],
    ),
    (
        "<a href=\"#\">x</a>",
        &[("a11y_invalid_attribute", 3, 11, false)],
    ),
    ("<a href=\"&amp;\">x</a>", &[]),
    ("<video src=\"a\" aria-hidden=\"tru&#101;\"></video>", &[]),
    ("<input type=\"text\" autocomplete=\"&#110;ame\">", &[]),
    (
        "<input type=\"text\" autocomplete=\"nam\">",
        &[("a11y_autocomplete_valid", 19, 37, false)],
    ),
    (
        "<div aria-hidden=\"tr&#117;e\" tabindex=\"0\">x</div>",
        &[("a11y_no_noninteractive_tabindex", 0, 49, false)],
    ),
];

#[test]
fn lint_reports_what_the_compiler_reports() {
    let mut failures = Vec::new();
    for &(src, expected) in CASES {
        let warnings =
            svn_lint::lint_file(src, Path::new("T.svelte"), None, CompatFeatures::MODERN);
        // The compiler stops at its first error.
        let mut got: Vec<Reported> = match warnings.iter().find(|w| w.is_error) {
            Some(error) => vec![(
                error.code.as_str(),
                error.range.start,
                error.range.end,
                true,
            )],
            None => warnings
                .iter()
                .map(|w| (w.code.as_str(), w.range.start, w.range.end, false))
                .collect(),
        };
        let mut want = expected.to_vec();
        got.sort_unstable();
        want.sort_unstable();
        if got != want {
            failures.push(format!("{src:?}\n  got:  {got:?}\n  want: {want:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
