//! Regression: `state_referenced_locally` inside an optional-chain
//! call (`obj?.meth(propRead)`). The scope walker was missing the
//! `Expression::ChainExpression` variant in `visit_expr`, so neither
//! the call's callee nor its arguments were being recorded as
//! references — any prop / state read that sat inside an optional
//! chain slipped through silently.

use std::path::Path;

fn lint(source: &str) -> Vec<svn_lint::Warning> {
    svn_lint::lint_file(source, Path::new("t.svelte"), Some(true))
}

fn positions(source: &str) -> Vec<(u32, u32)> {
    let mut v: Vec<(u32, u32)> = lint(source)
        .into_iter()
        .filter(|w| w.code == svn_lint::Code::state_referenced_locally)
        .map(|w| (w.start_line, w.start_column))
        .collect();
    v.sort();
    v
}

#[test]
fn prop_read_inside_optional_chain_call_arg_fires() {
    let src = "\
<script>
  let { data, appContext } = $props();
  if (data.team) {
    appContext?.videoUploader.setTeamId(data.team.id);
  }
</script>
";
    // Upstream-parity positions (source-sorted): if-cond `data`,
    // `appContext` callee, `data` inside the call argument.
    assert_eq!(positions(src), vec![(3, 6), (4, 4), (4, 40)]);
}

#[test]
fn prop_read_via_optional_member_chain_fires() {
    // No call — just `obj?.prop` — still needs the callee to be
    // visited. The read is `data`, col 2 on line 3.
    let src = "\
<script>
  let { data } = $props();
  data?.team.id;
</script>
";
    assert_eq!(positions(src), vec![(3, 2)]);
}

/// `// svelte-ignore …` comment placed *between* a call's `(` and its
/// argument expression must silence the rule for references inside the
/// argument. Upstream attaches leading comments per-node; our per-
/// statement capture wasn't enough on its own.
#[test]
fn svelte_ignore_inside_state_call_arg_suppresses() {
    let src = "\
<script>
  let { channel } = $props();
  let activeTab = $state(
    // svelte-ignore state_referenced_locally
    channel.radius?.value || 'rounded',
  );
</script>
";
    let warnings = positions(src);
    assert!(
        warnings.is_empty(),
        "svelte-ignore on the argument must suppress the warning, got: {warnings:?}"
    );
}

/// Negative case — missing ignore still fires. Guards against the
/// fix silencing everything.
#[test]
fn no_ignore_inside_state_call_arg_still_fires() {
    let src = "\
<script>
  let { channel } = $props();
  let activeTab = $state(channel.radius?.value || 'rounded');
</script>
";
    let warnings = positions(src);
    assert!(
        !warnings.is_empty(),
        "without svelte-ignore, the rule must fire — no regressions to the suppression path"
    );
}
