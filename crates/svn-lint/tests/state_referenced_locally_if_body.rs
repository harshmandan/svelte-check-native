//! Regression: `state_referenced_locally` must fire on prop reads
//! inside the body of a top-level `if`/`else` (or plain block) in the
//! instance script. An `if`-statement body is NOT a new function
//! scope, so references inside are still at the same function-depth
//! as the prop binding and must flag.
//!
//! Symptom observed on a bench workspace: upstream fired on both the
//! `if (data)` condition AND the `data.foo` read inside the body;
//! our linter only caught the condition.

use std::path::Path;

fn lint(source: &str) -> Vec<svn_lint::Warning> {
    svn_lint::lint_file(source, Path::new("t.svelte"), Some(true), svn_lint::CompatFeatures::MODERN)
}

fn state_locally(source: &str) -> Vec<(u32, u32)> {
    lint(source)
        .into_iter()
        .filter(|w| w.code == svn_lint::Code::state_referenced_locally)
        .map(|w| (w.start_line, w.start_column))
        .collect()
}

#[test]
fn prop_read_inside_if_body_fires() {
    let src = "\
<script>
  let { data } = $props();
  if (data) {
    console.log(data.foo);
  }
</script>
";
    let got = state_locally(src);
    // Upstream fires at 3:6 (if-condition) and 4:16 (body read).
    assert_eq!(
        got,
        vec![(3, 6), (4, 16)],
        "expected warnings at 3:6 and 4:16 (upstream parity)"
    );
}

#[test]
fn prop_read_inside_else_body_fires() {
    let src = "\
<script>
  let { data } = $props();
  if (!data) {
    // nothing
  } else {
    console.log(data);
  }
</script>
";
    let got = state_locally(src);
    assert_eq!(got.len(), 2, "expected two warnings (if-cond + else-body), got {got:?}");
}

#[test]
fn prop_read_inside_plain_block_fires() {
    let src = "\
<script>
  let { data } = $props();
  {
    console.log(data);
  }
</script>
";
    let got = state_locally(src);
    assert_eq!(got.len(), 1, "expected one warning (plain block body), got {got:?}");
}

#[test]
fn prop_read_inside_function_does_not_fire() {
    // Regression: make sure we don't over-fire — function bodies
    // bump function_depth and are safe.
    let src = "\
<script>
  let { data } = $props();
  function handler() {
    console.log(data);
  }
</script>
";
    let got = state_locally(src);
    assert!(got.is_empty(), "expected no warnings inside function body, got {got:?}");
}
