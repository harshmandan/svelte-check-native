//! Regression tests for scope-walker holes and rule-gate behavior
//! landed during the cross-bench parity push.
//!
//! Each `#[test]` here corresponds to a specific false-positive or
//! false-negative surfaced by running native lint against a real
//! bench workspace. Keeping them focused (one shape per test) makes
//! the next regression obvious.

use std::path::Path;
use svn_lint::{Code, CompatFeatures, SvelteVersion, Warning};

fn lint(source: &str, compat: CompatFeatures) -> Vec<Warning> {
    svn_lint::lint_file(source, Path::new("t.svelte"), Some(true), compat)
}

fn codes(warnings: &[Warning]) -> Vec<&str> {
    warnings.iter().map(|w| w.code.as_str()).collect()
}

// ----------------------------------------------------------------
// Scope-walker: spread elements / spread properties / shorthand
// directives / chain expressions / paren-wrapped svelte-ignore
// ----------------------------------------------------------------

/// `[...(cond ? [prop] : [])]` — the SpreadElement's argument was
/// silently dropped by the array walker because `as_expression()`
/// returns None for spreads. Props used only via spread looked
/// unused to `export_let_unused`.
#[test]
fn array_spread_element_argument_is_walked() {
    let src = "\
<script lang=\"ts\">
  export let things: number[] = []
  $: spread = [...((things.length) ? things : [])]
</script>

<p>{spread}</p>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "`things` used inside array spread must not fire export_let_unused, got: {:?}",
        codes(&warnings)
    );
}

/// `{ ...rest }` — same issue on ObjectExpression spread.
#[test]
fn object_spread_property_argument_is_walked() {
    let src = "\
<script lang=\"ts\">
  export let base: Record<string, number> = {}
  const merged = { ...base, x: 1 }
  void merged
</script>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "`base` in object spread must not fire, got: {:?}",
        codes(&warnings)
    );
}

/// `class:foo` without a value is shorthand for `class:foo={foo}` —
/// an implicit identifier read of `foo` in the current scope.
#[test]
fn class_directive_shorthand_records_reference() {
    let src = "\
<script lang=\"ts\">
  export let active = false
</script>

<div class:active />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "class:active shorthand must record a ref to `active`, got: {:?}",
        codes(&warnings)
    );
}

/// `style:foo` without a value is shorthand for `style:foo={foo}`.
#[test]
fn style_directive_shorthand_records_reference() {
    let src = "\
<script lang=\"ts\">
  export let color = 'red'
</script>

<div style:color />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(!codes(&warnings).contains(&"export_let_unused"));
}

/// `use:action`, `transition:fn`, `in:fn`, `out:fn`, `animate:fn` —
/// the directive NAME itself is a function reference. Not recording
/// it makes locally-declared transition/action functions look unused.
#[test]
fn transition_and_use_directives_record_their_function_name() {
    let src = "\
<script lang=\"ts\">
  import { fade } from 'svelte/transition'
  export let myAction: (node: Element) => void = () => {}
  export let myTransition = fade
</script>

<div use:myAction in:myTransition />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "use:/in: directive names must count as identifier reads, got: {:?}",
        codes(&warnings)
    );
}

/// `(/* svelte-ignore CODE */ expr)` — leading svelte-ignore
/// comments attached to the *inner* expression of a parenthesized
/// group were invisible to our per-statement ignore-stack scan.
#[test]
fn svelte_ignore_inside_parens_suppresses_inner_refs() {
    let src = "\
<script lang=\"ts\">
  let { data } = $props()
  const wrapped = (
    // svelte-ignore state_referenced_locally
    data.foo
  )
  void wrapped
</script>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "svelte-ignore inside parens must suppress, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Template-expression walk: object-literal attribute values
// ----------------------------------------------------------------

/// `use:foo={{ a: b, c: d }}` — the attribute-value expression
/// starts with `{`, which at JS program-body level parses as
/// BlockStatement and rejects `a: b, c: d` with a parse error.
/// `walk_expr_range` needs to wrap slices starting with `{` in
/// parens before parsing.
#[test]
fn template_expression_starting_with_brace_parses_as_expression() {
    let src = "\
<script lang=\"ts\">
  export let allowReorder = false
</script>

<div use:dndzone={{ dragDisabled: !allowReorder, items: [] }} />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "prop referenced inside `use:fn={{{{...}}}}` must count as used, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// attribute_global_event_reference: scope visibility
// ----------------------------------------------------------------

/// `<button {onclick}>` where `onclick` is a snippet parameter (not
/// an instance-script binding) must not fire
/// `attribute_global_event_reference`. The rule previously only
/// consulted `instance_root`; snippet-local declarations were
/// invisible.
#[test]
fn attribute_global_event_suppressed_by_snippet_parameter_binding() {
    let src = "\
<script lang=\"ts\">
  let { items }: { items: string[] } = $props()
</script>

{#snippet row(onclick: () => void)}
  <button {onclick}>ok</button>
{/snippet}

{#each items as _}
  {@render row(() => {})}
{/each}
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_global_event_reference"),
        "snippet param `onclick` should suppress the global-event warning, got: {:?}",
        codes(&warnings)
    );
}

/// Negative case — make sure the rule still fires when NO binding
/// named `onclick` exists anywhere.
#[test]
fn attribute_global_event_still_fires_on_truly_missing_binding() {
    let src = "<button {onclick}>ok</button>";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_global_event_reference"),
        "with no `onclick` binding anywhere, the rule must fire, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Version-gated rule behavior (compat flags)
// ----------------------------------------------------------------

/// Pre-5.45.3 svelte: `state_referenced_locally` did not fire on
/// regular `prop` bindings. Gated via
/// `compat.state_locally_fires_on_props`.
#[test]
fn state_referenced_locally_pre_5_45_3_does_not_fire_on_prop() {
    let src = "\
<script lang=\"ts\">
  let { foo } = $props()
  const x = foo
  void x
</script>
";
    let compat = CompatFeatures::from_version(Some(SvelteVersion {
        major: 5, minor: 45, patch: 2,
    }));
    let warnings = lint(src, compat);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "svelte 5.45.2 must not fire state_referenced_locally on `prop`, got: {:?}",
        codes(&warnings)
    );
}

/// Right at the 5.45.3 threshold the rule turns on.
#[test]
fn state_referenced_locally_5_45_3_fires_on_prop() {
    let src = "\
<script lang=\"ts\">
  let { foo } = $props()
  const x = foo
  void x
</script>
";
    let compat = CompatFeatures::from_version(Some(SvelteVersion {
        major: 5, minor: 45, patch: 3,
    }));
    let warnings = lint(src, compat);
    assert!(codes(&warnings).contains(&"state_referenced_locally"));
}

/// Pre-5.51.2 svelte: rest-prop bindings don't trip the rule even
/// though regular props do. Gated via
/// `compat.state_locally_rest_prop`. Fixture reads only the
/// rest-prop (not the regular prop) so the assertion isolates
/// that gate.
#[test]
fn state_referenced_locally_pre_5_51_2_does_not_fire_on_rest_prop() {
    let src = "\
<script lang=\"ts\">
  const { ...rest } = $props()
  const x = rest
  void x
</script>
";
    let compat = CompatFeatures::from_version(Some(SvelteVersion {
        major: 5, minor: 51, patch: 1,
    }));
    let warnings = lint(src, compat);
    assert_eq!(
        warnings
            .iter()
            .filter(|w| w.code == Code::state_referenced_locally)
            .count(),
        0,
        "pre-5.51.2 must not fire on rest-prop reads, got: {:?}",
        codes(&warnings)
    );
}

/// 5.51.2+ fires on rest-prop reads too.
#[test]
fn state_referenced_locally_5_51_2_fires_on_rest_prop_read() {
    let src = "\
<script lang=\"ts\">
  const { foo, ...rest } = $props()
  const x = rest
  void foo
  void x
</script>
";
    let compat = CompatFeatures::from_version(Some(SvelteVersion {
        major: 5, minor: 51, patch: 2,
    }));
    let warnings = lint(src, compat);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "svelte 5.51.2 should fire state_referenced_locally on rest-prop reads, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// runes-mode inference: call-shape required
// ----------------------------------------------------------------

/// A bare substring like `$$props` (Svelte-4 ambient store) that
/// happens to contain `$props` must not flip runes-mode inference.
/// Previously `source.contains("$props")` matched inside `$$props`,
/// incorrectly promoting non-runes files to runes mode and firing
/// `event_directive_deprecated` / `slot_element_deprecated` /
/// `script_context_deprecated` on legitimate Svelte-4 code.
#[test]
fn runes_inference_ignores_double_dollar_props_substring() {
    // No rune CALLS — only a Svelte-4 `$$props` reference.
    let src = "\
<script>
  export let foo = 1
</script>

<div>{$$props.class}</div>
";
    // Pass None for runes so inference runs.
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    let codes: Vec<&str> = warnings.iter().map(|w| w.code.as_str()).collect();
    assert!(
        !codes.contains(&"event_directive_deprecated"),
        "$$props must not promote the file to runes mode, got: {codes:?}"
    );
}
