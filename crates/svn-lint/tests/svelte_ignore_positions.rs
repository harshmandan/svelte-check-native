//! `// svelte-ignore` position matrix for script bodies.
//!
//! Upstream attaches comments to AST nodes via its acorn comment
//! handlers (`phases/1-parse/acorn.js::add_comments`): every queued
//! comment becomes a `leadingComment` of the next node starting after
//! it — regardless of stacking or blank lines — unless it is consumed
//! as a `trailingComment` of the previous node (same line, separated
//! only by `,`, `)`, spaces, tabs). The analyze walk then honors the
//! ignores of ALL leading comments at EVERY node (statements,
//! call/new arguments, object properties, array elements, …).
//!
//! Every case in this file was verified against the real Svelte
//! compiler. The "keeps warning" cases lock the trailing-comment rule
//! so the leading-comment fix cannot over-suppress.

use std::path::Path;
use svn_lint::{Code, CompatFeatures, Warning};

fn lint(source: &str) -> Vec<Warning> {
    svn_lint::lint_file(
        source,
        Path::new("t.svelte"),
        Some(true),
        CompatFeatures::MODERN,
    )
}

fn codes(warnings: &[Warning]) -> Vec<&str> {
    warnings.iter().map(|w| w.code.as_str()).collect()
}

fn assert_suppressed(src: &str, note: &str) {
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"state_referenced_locally"),
        "{note}: svelte-ignore must suppress state_referenced_locally, got: {:?}",
        codes(&warnings)
    );
}

fn assert_warns(src: &str, note: &str) {
    let warnings = lint(src);
    assert!(
        codes(&warnings).contains(&"state_referenced_locally"),
        "{note}: state_referenced_locally must still fire, got: {:?}",
        codes(&warnings)
    );
}

// ----------------------------------------------------------------
// Suppressed positions
// ----------------------------------------------------------------

/// Leading comment on an object literal property (issue repro).
#[test]
fn object_property_single_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);

\tconst sections = [
\t\t{
\t\t\tvalue: 'a',
\t\t\t// svelte-ignore state_referenced_locally
\t\t\topen: count > 0
\t\t}
\t];
</script>

<button onclick={() => count++}>{sections[0].open} / {count}</button>
",
        "object property, single comment",
    );
}

/// Stacked comments before a `new` expression argument (issue repro).
#[test]
fn new_expression_argument_stacked_comments() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet { projectId, criterion }: { projectId: string; criterion: string } = $props();

\tclass Persisted<T> {
\t\tconstructor(
\t\t\tpublic key: string,
\t\t\tpublic value: T
\t\t) {}
\t}

\tconst thing = new Persisted<{ title: string }>(
\t\t// eslint-disable-next-line svelte/no-unused-svelte-ignore
\t\t// svelte-ignore state_referenced_locally
\t\t`key-${projectId}-${criterion}`,
\t\t{ title: '' }
\t);
</script>

<p>{thing.key}</p>
",
        "new-expression argument, stacked comments",
    );
}

/// Single comment before a `new` expression argument — proves the
/// stacked variant above isn't about stacking.
#[test]
fn new_expression_argument_single_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tclass Box { constructor(public v: number) {} }
\tconst b = new Box(
\t\t// svelte-ignore state_referenced_locally
\t\tcount
\t);
\tvoid b;
</script>

<button onclick={() => count++}>{count}</button>
",
        "new-expression argument, single comment",
    );
}

/// Call argument, single comment (worked before the fix — lock it).
#[test]
fn call_argument_single_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tfunction take(n: number) { return n; }
\tconst t = take(
\t\t// svelte-ignore state_referenced_locally
\t\tcount
\t);
\tvoid t;
</script>

<button onclick={() => count++}>{count}</button>
",
        "call argument, single comment",
    );
}

/// Call argument, other comment stacked ABOVE the svelte-ignore.
#[test]
fn call_argument_ignore_below_other_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tfunction take(n: number) { return n; }
\tconst t = take(
\t\t// eslint-disable-next-line svelte/no-unused-svelte-ignore
\t\t// svelte-ignore state_referenced_locally
\t\tcount
\t);
\tvoid t;
</script>

<button onclick={() => count++}>{count}</button>
",
        "call argument, ignore below another comment",
    );
}

/// Call argument, svelte-ignore ABOVE another comment — the chain
/// must hop through the intervening non-ignore comment.
#[test]
fn call_argument_ignore_above_other_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tfunction take(n: number) { return n; }
\tconst t = take(
\t\t// svelte-ignore state_referenced_locally
\t\t// some unrelated note
\t\tcount
\t);
\tvoid t;
</script>

<button onclick={() => count++}>{count}</button>
",
        "call argument, ignore above another comment",
    );
}

/// Statement level, svelte-ignore ABOVE another comment — the
/// already-working statement path must also chain through runs.
#[test]
fn statement_ignore_above_other_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\t// svelte-ignore state_referenced_locally
\t// eslint-disable-next-line prefer-const
\tlet snapshot = count;
\tvoid snapshot;
</script>

<button onclick={() => count++}>{count}</button>
",
        "statement level, ignore above another comment",
    );
}

/// Array element.
#[test]
fn array_element_comment() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tconst arr = [
\t\t// svelte-ignore state_referenced_locally
\t\tcount
\t];
\tvoid arr;
</script>

<button onclick={() => count++}>{count}</button>
",
        "array element",
    );
}

/// Initializer on the line after `=` — the comment sits inside the
/// declaration statement, before the init expression.
#[test]
fn initializer_after_line_break() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tconst snapshot =
\t\t// svelte-ignore state_referenced_locally
\t\tcount;
\tvoid snapshot;
</script>

<button onclick={() => count++}>{count}</button>
",
        "initializer on the next line",
    );
}

/// Inline block comment between a property key's `:` and its value.
#[test]
fn inline_block_comment_before_property_value() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tconst obj = { open: /* svelte-ignore state_referenced_locally */ count > 0 };
\tvoid obj;
</script>

<button onclick={() => count++}>{count}</button>
",
        "inline block comment before property value",
    );
}

/// Trailing non-ignore comment then a same-line svelte-ignore block
/// comment: the plain comment is consumed as trailing of `'a'`, the
/// svelte-ignore still leads the next property.
#[test]
fn block_comments_after_previous_property() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tconst obj = {
\t\tvalue: 'a', /* note */ /* svelte-ignore state_referenced_locally */
\t\topen: count > 0
\t};
\tvoid obj;
</script>

<button onclick={() => count++}>{count}</button>
",
        "block-comment pair after previous property",
    );
}

/// Comment right after the call's `(` on the same line — nothing that
/// can end an expression precedes it, so it leads the first argument.
#[test]
fn comment_after_open_paren_same_line() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tfunction take(n: number) { return n; }
\tconst t = take( // svelte-ignore state_referenced_locally
\t\tcount
\t);
\tvoid t;
</script>

<button onclick={() => count++}>{count}</button>
",
        "comment after open paren",
    );
}

/// Blank line between the comment and the statement — upstream
/// attaches across blank lines.
#[test]
fn blank_line_between_comment_and_statement() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\t// svelte-ignore state_referenced_locally

\tconst snapshot = count;
\tvoid snapshot;
</script>

<button onclick={() => count++}>{count}</button>
",
        "blank line between comment and statement",
    );
}

/// Class property definition initializer.
#[test]
fn class_property_initializer() {
    assert_suppressed(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tclass Holder {
\t\t// svelte-ignore state_referenced_locally
\t\tinitial = count;
\t}
\tvoid new Holder();
</script>

<button onclick={() => count++}>{count}</button>
",
        "class property initializer",
    );
}

// ----------------------------------------------------------------
// Positions that must KEEP warning (trailing-comment rule)
// ----------------------------------------------------------------

/// No comment at all — baseline.
#[test]
fn no_comment_baseline_warns() {
    assert_warns(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tconst snapshot = count;
\tvoid snapshot;
</script>

<button onclick={() => count++}>{count}</button>
",
        "no comment",
    );
}

/// Same-line comment after the previous property is a TRAILING
/// comment of that property — it does not lead the next one.
#[test]
fn trailing_comment_after_previous_property_warns() {
    assert_warns(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tconst obj = {
\t\tvalue: 'a', // svelte-ignore state_referenced_locally
\t\topen: count > 0
\t};
\tvoid obj;
</script>

<button onclick={() => count++}>{count}</button>
",
        "trailing comment after previous property",
    );
}

/// Same-line comment after a previous argument is trailing of that
/// argument, not leading of the next.
#[test]
fn trailing_comment_after_previous_argument_warns() {
    assert_warns(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tfunction take2(a: number, b: number) { return a + b; }
\tconst t = take2(1, // svelte-ignore state_referenced_locally
\t\tcount
\t);
\tvoid t;
</script>

<button onclick={() => count++}>{count}</button>
",
        "trailing comment after previous argument",
    );
}

/// Comment after a statement's closing paren is trailing of that
/// statement — the next statement is not suppressed.
#[test]
fn trailing_comment_after_statement_warns() {
    assert_warns(
        "<script lang=\"ts\">
\tlet count = $state(0);
\tfunction noop() {}
\tnoop() // svelte-ignore state_referenced_locally
\tconst snapshot = count;
\tvoid snapshot;
</script>

<button onclick={() => count++}>{count}</button>
",
        "trailing comment after previous statement",
    );
}

// ----------------------------------------------------------------
// Binding-anchored rules (warning fires at the declaration)
// ----------------------------------------------------------------

/// `non_reactive_update` honors a leading svelte-ignore across a
/// blank line (the old line-scanner broke the chain on blank lines).
#[test]
fn non_reactive_update_ignore_across_blank_line() {
    let src = "<script lang=\"ts\">
\t// svelte-ignore non_reactive_update

\tlet value = 'a';
\tfunction set() { value = 'b'; }
</script>

<p>{value}</p><button onclick={set}>x</button>
";
    let warnings = lint(src);
    assert!(
        !codes(&warnings).contains(&"non_reactive_update"),
        "ignore across blank line must suppress non_reactive_update, got: {:?}",
        codes(&warnings)
    );
}

/// `export_let_unused` (non-runes) honors a leading svelte-ignore on
/// the export declaration.
#[test]
fn export_let_unused_leading_ignore() {
    let src = "<script>
\t// svelte-ignore export_let_unused
\texport let unused = 1;
</script>

<p>hi</p>
";
    let warnings = svn_lint::lint_file(
        src,
        Path::new("t.svelte"),
        Some(false),
        CompatFeatures::MODERN,
    );
    assert!(
        !codes(&warnings).contains(&"export_let_unused"),
        "leading ignore must suppress export_let_unused, got: {:?}",
        codes(&warnings)
    );
    let _ = Code::export_let_unused;
}
