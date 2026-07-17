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

// ----------------------------------------------------------------
// attribute_invalid_property_name: component props vs DOM attrs
// ----------------------------------------------------------------

#[test]
fn react_style_attribute_name_is_allowed_on_component_prop() {
    let src = "\
<script>
  import Favicon from './Favicon.svelte'
</script>

<Favicon className=\"text-muted\" />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_invalid_property_name"),
        "component prop `className` must not fire DOM attr warning, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_attribute_name_still_warns_on_dom_element() {
    let src = "<div className=\"text-muted\"></div>";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_invalid_property_name"),
        "DOM `className` should still warn, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_expression_attribute_name_is_allowed_on_component_prop() {
    let src = "\
<script>
  import Favicon from './Favicon.svelte'
  let className = 'text-muted'
</script>

<Favicon className={className} />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_invalid_property_name"),
        "component prop expression `className` must not fire DOM attr warning, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_expression_attribute_name_still_warns_on_dom_element() {
    let src = "\
<script>
  let className = 'text-muted'
</script>

<div className={className}></div>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_invalid_property_name"),
        "DOM expression `className` should still warn, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_shorthand_attribute_name_is_allowed_on_component_prop() {
    let src = "\
<script>
  import Favicon from './Favicon.svelte'
  let className = 'text-muted'
</script>

<Favicon {className} />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_invalid_property_name"),
        "component prop shorthand `className` must not fire DOM attr warning, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_shorthand_attribute_name_still_warns_on_dom_element() {
    let src = "\
<script>
  let className = 'text-muted'
</script>

<div {className}></div>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_invalid_property_name"),
        "DOM shorthand `className` should still warn, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_attribute_name_still_warns_on_custom_element() {
    let src = "<my-widget className=\"text-muted\"></my-widget>";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_invalid_property_name"),
        "custom-element `className` should still warn, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_attribute_name_is_allowed_on_svelte_component_prop() {
    let src = "\
<script>
  import Favicon from './Favicon.svelte'
</script>

<svelte:component this={Favicon} className=\"text-muted\" />
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_invalid_property_name"),
        "`svelte:component` prop `className` must not fire DOM attr warning, got: {:?}",
        codes(&warnings)
    );
}

#[test]
fn react_style_attribute_name_still_warns_on_svelte_element() {
    let src = "<svelte:element this=\"div\" className=\"text-muted\" />";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_invalid_property_name"),
        "`svelte:element` `className` should still warn, got: {:?}",
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
        major: 5,
        minor: 45,
        patch: 2,
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
        major: 5,
        minor: 45,
        patch: 3,
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
        major: 5,
        minor: 51,
        patch: 1,
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
        major: 5,
        minor: 51,
        patch: 2,
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

// ----------------------------------------------------------------
// Template <!-- svelte-ignore --> → instance script bridge
// ----------------------------------------------------------------

/// `<!-- svelte-ignore CODE -->` placed between the module and
/// instance scripts applies its codes to the whole instance script
/// body. Upstream wires this up because the script is an AST
/// sibling inside the root Fragment; our sections parser extracts
/// it separately, so the ignore has to be bridged explicitly.
#[test]
fn template_comment_before_script_suppresses_script_ignores() {
    let src = "\
<script module>
  const base = 1
</script>

<!-- svelte-ignore state_referenced_locally -->
<script lang=\"ts\">
  let { foo } = $props()
  const x = foo
  void x
</script>
";
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "template svelte-ignore before <script> must suppress the script's fires, got: {:?}",
        codes(&warnings)
    );
}

/// With stacked `<!-- svelte-ignore A --><!-- svelte-ignore B -->`
/// comments only the NEAREST one bridges into the script — upstream's
/// parser takes a single `prev_comment` (element.js) and breaks at
/// the first comment scanning backward. Verified against svelte
/// 5.56.5: the outer comment's code still warns.
#[test]
fn only_nearest_template_comment_applies_to_script() {
    let src = "\
<!-- svelte-ignore state_referenced_locally -->
<!-- svelte-ignore non_reactive_update -->
<script lang=\"ts\">
  let { foo } = $props()
  const x = foo
  void x
</script>
";
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "only the nearest comment bridges; the outer one must still warn, got: {:?}",
        codes(&warnings)
    );
}

/// The nearest-comment bridge applies to a MODULE script too.
#[test]
fn template_comment_bridges_into_module_script() {
    let src = "\
<p>lead</p>
<!-- svelte-ignore bidirectional_control_characters -->
<script module>
\tconst x = '\u{202a}hidden';
\tvoid x;
</script>
<p>hi</p>
";
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"bidirectional_control_characters"),
        "template ignore must bridge into the module script, got: {:?}",
        codes(&warnings)
    );
}

/// A comment body containing a literal `<!--` still parses — the
/// pairing must come from the parsed Comment node, not a textual
/// backward scan for the opener.
#[test]
fn template_comment_with_embedded_opener_still_bridges() {
    let src = "\
<p>lead</p>
<!-- svelte-ignore bidirectional_control_characters <!-- x -->
<script>
\tconst y = '\u{202a}hidden';
\tvoid y;
</script>
<p>hi</p>
";
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"bidirectional_control_characters"),
        "embedded <!-- must not break the ignore comment, got: {:?}",
        codes(&warnings)
    );
}

/// A comma-separated list inside a single comment must also work.
#[test]
fn comma_list_in_template_comment_applies_to_script() {
    let src = "\
<!-- svelte-ignore state_referenced_locally, non_reactive_update -->
<script lang=\"ts\">
  let { foo } = $props()
  const x = foo
  void x
</script>
";
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "comma-list ignore must apply, got: {:?}",
        codes(&warnings)
    );
}

/// A comment separated from the script by non-whitespace template
/// content does NOT apply to the script.
#[test]
fn template_comment_separated_from_script_does_not_leak_in() {
    let src = "\
<!-- svelte-ignore state_referenced_locally -->
<p>some real content</p>
<script lang=\"ts\">
  let { foo } = $props()
  const x = foo
  void x
</script>
";
    let warnings = svn_lint::lint_file(src, Path::new("t.svelte"), None, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "comment separated by a <p> must not bridge to the script, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Runes-mode inference
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

/// `<svelte:element this={…dynamic…} onclick>` must fire
/// `a11y_no_static_element_interactions`. Upstream's check_element
/// does NOT gate this rule on `is_dynamic_element`, so dynamic
/// elements with an interactive handler and no resolvable role
/// still warn. Our pre-fix guard skipped the rule entirely whenever
/// `this` wasn't a string literal.
#[test]
fn svelte_element_dynamic_this_with_onclick_fires_no_static_element_interactions() {
    let src = "\
<script>
  let onclick = () => {}
  let isAnchor = false
</script>

<svelte:element this={isAnchor ? 'a' : 'div'} {onclick}>x</svelte:element>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    let cs = codes(&warnings);
    assert!(
        cs.contains(&"a11y_no_static_element_interactions"),
        "dynamic <svelte:element> with onclick must fire a11y_no_static_element_interactions, got: {cs:?}"
    );
}

/// Same shape with a `svelte-ignore` comment above must NOT fire —
/// the suppression path goes through the same emit channel, so
/// removing the dynamic gate must not bypass it.
#[test]
fn svelte_element_dynamic_this_with_onclick_respects_svelte_ignore() {
    let src = "\
<script>
  let onclick = () => {}
  let isAnchor = false
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<svelte:element this={isAnchor ? 'a' : 'div'} {onclick}>x</svelte:element>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    let cs = codes(&warnings);
    assert!(
        !cs.contains(&"a11y_no_static_element_interactions"),
        "svelte-ignore must suppress the warning, got: {cs:?}"
    );
}

// ----------------------------------------------------------------
// l275: attribute_global_event_reference resolves the on-event
// identifier against the element's LEXICAL scope (upstream
// shared/element.js `scope.get`), not the whole-file declaration set.
// ----------------------------------------------------------------

/// `{onclick}` where `onclick` is declared only in a non-enclosing
/// each scope must FIRE — it's not in lexical scope at the button.
#[test]
fn global_event_ref_fires_when_binding_in_unrelated_scope() {
    let src = "{#each a as onclick}{/each}\n<button {onclick}>x</button>\n";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"attribute_global_event_reference"),
        "onclick declared only in a non-enclosing each scope must fire, got: {:?}",
        codes(&warnings)
    );
}

/// Reverse-legit guard: a button INSIDE the each (onclick IS in
/// lexical scope) must NOT fire.
#[test]
fn global_event_ref_suppressed_when_binding_encloses_element() {
    let src = "{#each a as onclick}<button {onclick}>x</button>{/each}\n";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_global_event_reference"),
        "onclick in the enclosing each scope must suppress, got: {:?}",
        codes(&warnings)
    );
}

/// A top-level (instance/template-root) binding still suppresses.
#[test]
fn global_event_ref_suppressed_by_top_level_binding() {
    let src =
        "<script lang=\"ts\">let onclick = () => {};</script>\n<button {onclick}>x</button>\n";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"attribute_global_event_reference"),
        "top-level onclick must suppress, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Scope-walker: spread call/new arguments, for-of/for-in left,
// catch params, class constructs, assignment-target destructuring,
// var hoisting, named function expressions.
//
// Every expectation below was verified against the real Svelte
// compiler (5.56.5).
// ----------------------------------------------------------------

fn lint_nonrunes(source: &str) -> Vec<Warning> {
    svn_lint::lint_file(
        source,
        Path::new("t.svelte"),
        Some(false),
        CompatFeatures::MODERN,
    )
}

/// `f(...a)` — the spread argument's identifier is a reference, so
/// the prop is used (upstream: no export_let_unused).
#[test]
fn call_spread_argument_counts_as_usage() {
    let src = "\
<script>
  export let a;
  function f(...xs) { void xs; }
  f(...a);
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "prop used via call spread arg must not fire, got: {:?}",
        codes(&warnings)
    );
}

/// `new F(...a)` — same for new-expression spread arguments.
#[test]
fn new_spread_argument_counts_as_usage() {
    let src = "\
<script>
  export let a;
  class F { constructor(...xs) { this.xs = xs; } }
  const f = new F(...a);
  void f;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "prop used via new spread arg must not fire, got: {:?}",
        codes(&warnings)
    );
}

/// A `$state` read inside a spread argument is a local reference —
/// upstream fires state_referenced_locally on `f(...[count])`.
#[test]
fn call_spread_argument_records_state_read() {
    let src = "\
<script>
  let count = $state(0);
  function f(...xs) { void xs; }
  f(...[count]);
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "state read inside call spread arg must fire, got: {:?}",
        codes(&warnings)
    );
}

/// `for (const count of list)` declares `count` in a fresh block
/// scope — body reads resolve to the loop variable, not the outer
/// $state binding (upstream: no state_referenced_locally).
#[test]
fn for_of_declaration_shadows_outer_state() {
    let src = "\
<script>
  let count = $state(0);
  const list = [1, 2];
  for (const count of list) { console.log(count); }
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "for-of loop variable must shadow the outer state, got: {:?}",
        codes(&warnings)
    );
}

/// Control: a for-of body reading the OUTER state still fires.
#[test]
fn for_of_body_read_of_outer_state_fires() {
    let src = "\
<script>
  let count = $state(0);
  const list = [1, 2];
  for (const item of list) { console.log(item, count); }
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "outer-state read in for-of body must fire, got: {:?}",
        codes(&warnings)
    );
}

/// `for (x of xs)` with an identifier left is NOT a reassignment
/// upstream (updates come only from AssignmentExpression /
/// UpdateExpression) — verified: no non_reactive_update.
#[test]
fn for_of_identifier_left_is_not_a_reassignment() {
    let src = "\
<script>
  let x = 'a';
  const xs = ['b', 'c'];
  function go() { for (x of xs) { console.log(x); } }
</script>
<p>{x}</p><button onclick={go}>go</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"non_reactive_update"),
        "for-of identifier left must not count as reassignment, got: {:?}",
        codes(&warnings)
    );
}

/// A catch parameter shadows the outer binding inside the handler.
#[test]
fn catch_param_shadows_outer_state() {
    let src = "\
<script>
  let count = $state(0);
  try { console.log('x'); } catch (count) { console.log(count); }
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "catch param must shadow the outer state, got: {:?}",
        codes(&warnings)
    );
}

/// Control: a catch body reading the OUTER state still fires.
#[test]
fn catch_body_read_of_outer_state_fires() {
    let src = "\
<script>
  let count = $state(0);
  try { console.log('x'); } catch (e) { console.log(e, count); }
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "outer-state read in catch body must fire, got: {:?}",
        codes(&warnings)
    );
}

/// `class Foo extends Base {}` — the super-class is a reference.
#[test]
fn class_super_class_counts_as_usage() {
    let src = "\
<script>
  export let Base;
  class Foo extends Base {}
  void Foo;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "super-class reference must count as usage, got: {:?}",
        codes(&warnings)
    );
}

/// A class-expression property initializer reading `$state` fires
/// state_referenced_locally, same as a class declaration.
#[test]
fn class_expression_property_init_records_state_read() {
    let src = "\
<script>
  let count = $state(0);
  const C = class { x = count; };
  void C;
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "class-expression property init must record the read, got: {:?}",
        codes(&warnings)
    );
}

/// A static block does NOT bump function depth upstream (scope.js has
/// no StaticBlock visitor) — a `$state` read inside fires.
#[test]
fn class_static_block_records_state_read() {
    let src = "\
<script>
  let count = $state(0);
  class A { static { console.log(count); } }
  void A;
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "static-block read must fire at the declaration depth, got: {:?}",
        codes(&warnings)
    );
}

/// A computed class-member key is a reference.
#[test]
fn class_computed_key_counts_as_usage() {
    let src = "\
<script>
  export let k;
  class A { [k]() {} }
  void A;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "computed key must count as usage, got: {:?}",
        codes(&warnings)
    );
}

/// `import(path)` — the dynamic-import argument is a reference.
#[test]
fn dynamic_import_argument_counts_as_usage() {
    let src = "\
<script>
  export let path;
  function load() { return import(path); }
  void load;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "dynamic import argument must count as usage, got: {:?}",
        codes(&warnings)
    );
}

/// `#x in obj` — the right side of a private-in test is a reference.
#[test]
fn private_in_expression_counts_as_usage() {
    let src = "\
<script>
  export let obj;
  class A { #x = 1; m() { return #x in obj; } }
  void A;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "private-in right side must count as usage, got: {:?}",
        codes(&warnings)
    );
}

/// Upstream scope.js has NO ClassExpression id handling — the name
/// of a named class expression resolves OUTWARD (verified: the body
/// reference counts as prop usage, no export_let_unused).
#[test]
fn named_class_expression_name_resolves_outward() {
    let src = "\
<script>
  export let Inner;
  const C = class Inner { m() { return Inner; } };
  void C;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "named class expression must NOT self-declare its name, got: {:?}",
        codes(&warnings)
    );
}

/// A named function expression DOES declare its own name inside its
/// scope (scope.js FunctionExpression) — the body's `g()` resolves to
/// the function itself, so the prop stays unused (verified upstream).
#[test]
fn named_function_expression_self_declares() {
    let src = "\
<script>
  export let g;
  const f = function g() { g(); };
  void f;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        codes(&warnings).contains(&"export_let_unused"),
        "named fn expression must shadow the outer prop, got: {:?}",
        codes(&warnings)
    );
}

/// `[el] = pair` marks `el` reassigned (upstream unwrap_pattern on
/// AssignmentExpression left) — non_reactive_update fires.
#[test]
fn destructuring_assignment_marks_reassigned() {
    let src = "\
<script>
  let el;
  const pair = ['a'];
  function go() { [el] = pair; }
</script>
<p>{el}</p><button onclick={go}>go</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"non_reactive_update"),
        "destructuring assignment must mark the target reassigned, got: {:?}",
        codes(&warnings)
    );
}

/// Object-destructuring form too: `({ el } = obj)`.
#[test]
fn object_destructuring_assignment_marks_reassigned() {
    let src = "\
<script>
  let el;
  const obj = { el: 'a' };
  function go() { ({ el } = obj); }
</script>
<p>{el}</p><button onclick={go}>go</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"non_reactive_update"),
        "object destructuring assignment must mark reassigned, got: {:?}",
        codes(&warnings)
    );
}

/// `var` declarations hoist out of porous (block) scopes to the
/// nearest function/root scope (scope.js:675) — the template ref
/// resolves and non_reactive_update fires.
#[test]
fn var_hoists_out_of_block_scope() {
    let src = "\
<script>
  { var x = 'a'; }
  function go() { x = 'b'; }
</script>
<p>{x}</p><button onclick={go}>go</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"non_reactive_update"),
        "block-scoped var must hoist to the instance root, got: {:?}",
        codes(&warnings)
    );
}

/// `for (let count = 0; …)` — the init declaration lands in the
/// loop's own block scope, shadowing the outer state binding.
#[test]
fn for_init_declaration_shadows_outer_state() {
    let src = "\
<script>
  let count = $state(0);
  for (let count = 0; count < 2; count++) { console.log(count); }
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "for-init declaration must shadow the outer state, got: {:?}",
        codes(&warnings)
    );
}

/// Bidirectional control characters hide inside spread call
/// arguments too — walker coverage, verified upstream.
#[test]
fn bidi_string_inside_call_spread_argument_fires() {
    let src = "\
<script>
  function f(...xs) { void xs; }
  f(...['\u{202a}hidden']);
</script>
<p>hi</p>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"bidirectional_control_characters"),
        "bidi char inside call spread arg must fire, got: {:?}",
        codes(&warnings)
    );
}

/// Same for new-expression spread arguments.
#[test]
fn bidi_string_inside_new_spread_argument_fires() {
    let src = "\
<script>
  class F { constructor(...xs) { void xs; } }
  void new F(...['\u{202a}hidden']);
</script>
<p>hi</p>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"bidirectional_control_characters"),
        "bidi char inside new spread arg must fire, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Script-AST rule walker: enclosing-frame svelte-ignore lookups.
// Upstream pushes an ignore frame at EVERY node with leading
// comments and consults the whole stack, so an ignore above a
// statement suppresses warnings anchored anywhere in its subtree.
// Each case verified against svelte 5.56.5.
// ----------------------------------------------------------------

/// Ignore above `const x = "…"` — the warning anchors at the string
/// literal, but the statement-level comment must suppress it.
#[test]
fn bidi_ignore_above_declaration_suppresses() {
    let src = "\
<script>
\t// svelte-ignore bidirectional_control_characters
\tconst x = '\u{202a}hidden';
\tvoid x;
</script>
<p>hi</p>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"bidirectional_control_characters"),
        "statement-level ignore must suppress the literal-anchored bidi warning, got: {:?}",
        codes(&warnings)
    );
}

/// Ignore above a function declaration suppresses
/// perf_avoid_nested_class fired by a class nested in its body.
#[test]
fn perf_nested_class_ignore_above_function_suppresses() {
    let src = "\
<script>
\tlet count = $state(0);
\t// svelte-ignore perf_avoid_nested_class
\tfunction make() {
\t\tclass Inner {}
\t\treturn Inner;
\t}
\tvoid make;
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        !codes(&warnings).contains(&"perf_avoid_nested_class"),
        "function-level ignore must suppress the nested-class warning, got: {:?}",
        codes(&warnings)
    );
}

/// Control: without the ignore the nested class still fires.
#[test]
fn perf_nested_class_in_function_fires() {
    let src = "\
<script>
\tlet count = $state(0);
\tfunction make() {
\t\tclass Inner {}
\t\treturn Inner;
\t}
\tvoid make;
</script>
<button onclick={() => count++}>{count}</button>
";
    let warnings = lint(src, CompatFeatures::MODERN);
    assert!(
        codes(&warnings).contains(&"perf_avoid_nested_class"),
        "nested class in a function body must fire, got: {:?}",
        codes(&warnings)
    );
}

/// Upstream fires legacy_component_creation from its
/// ExpressionStatement visitor ONLY — `throw new App({target})` does
/// NOT warn (verified against the compiler).
#[test]
fn legacy_component_creation_does_not_fire_on_throw() {
    let src = "\
<script>
\timport App from './App.svelte';
\tthrow new App({ target: document.body });
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"legacy_component_creation"),
        "throw-new is not an expression statement upstream, got: {:?}",
        codes(&warnings)
    );
}

/// Control: the plain expression-statement form fires and its
/// statement-level ignore suppresses.
#[test]
fn legacy_component_creation_fires_and_ignores_at_statement() {
    let fires = "\
<script>
\timport App from './App.svelte';
\tnew App({ target: document.body });
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(fires);
    assert!(
        codes(&warnings).contains(&"legacy_component_creation"),
        "expression-statement form must fire, got: {:?}",
        codes(&warnings)
    );
    let ignored = "\
<script>
\timport App from './App.svelte';
\t// svelte-ignore legacy_component_creation
\tnew App({ target: document.body });
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(ignored);
    assert!(
        !codes(&warnings).contains(&"legacy_component_creation"),
        "statement-level ignore must suppress, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// bind_invalid_each_rest / store_rune_conflict ignore semantics.
// Verified against svelte 5.56.5.
// ----------------------------------------------------------------

/// A template `<!-- svelte-ignore bind_invalid_each_rest -->` above
/// the `{#each}` suppresses the warning (upstream fires it during
/// the walk, under the live ignore stack).
#[test]
fn bind_invalid_each_rest_template_ignore_suppresses() {
    let src = "\
<script>
\tlet items = [{ a: 1, b: 2 }];
</script>
<!-- svelte-ignore bind_invalid_each_rest -->
{#each items as { a, ...rest }}
\t<input bind:value={rest.b} />
\t{a}
{/each}
";
    let warnings = lint_nonrunes(src);
    assert!(
        !codes(&warnings).contains(&"bind_invalid_each_rest"),
        "template ignore above the each must suppress, got: {:?}",
        codes(&warnings)
    );
}

/// Control: without the comment the warning fires at the rest
/// binding's declaration.
#[test]
fn bind_invalid_each_rest_fires_without_ignore() {
    let src = "\
<script>
\tlet items = [{ a: 1, b: 2 }];
</script>
{#each items as { a, ...rest }}
\t<input bind:value={rest.b} />
\t{a}
{/each}
";
    let warnings = lint_nonrunes(src);
    assert!(
        codes(&warnings).contains(&"bind_invalid_each_rest"),
        "each-rest bind must fire, got: {:?}",
        codes(&warnings)
    );
}

/// store_rune_conflict fires BEFORE upstream's analyze walk, when the
/// ignore map is empty — neither a script `// svelte-ignore` nor a
/// template comment can suppress it (both verified).
#[test]
fn store_rune_conflict_is_unsuppressable() {
    let script_ignore = "\
<script>
\tlet state = 5;
\t// svelte-ignore store_rune_conflict
\tconst doubled = $state(5);
\tvoid doubled; void state;
</script>
<p>hi</p>
";
    let warnings = lint_nonrunes(script_ignore);
    assert!(
        codes(&warnings).contains(&"store_rune_conflict"),
        "script ignore must NOT suppress store_rune_conflict, got: {:?}",
        codes(&warnings)
    );
    let template_ignore = "\
<script>
\tlet state = 5;
\tvoid state;
</script>
<!-- svelte-ignore store_rune_conflict -->
<p>{$state(5)}</p>
";
    let warnings = lint_nonrunes(template_ignore);
    assert!(
        codes(&warnings).contains(&"store_rune_conflict"),
        "template ignore must NOT suppress store_rune_conflict, got: {:?}",
        codes(&warnings)
    );
}
