//! `{#snippet}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/SnippetBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::SnippetBlock;

use crate::emit_buffer::EmitBuffer;
use crate::nodes::inline_component::annotate_snippet_params;
use crate::{all_identifiers, emit_template_body};

/// Emit a lexical-scope block wrapping a `{#snippet name(params)}` body
/// so the snippet's parameter identifiers are in scope for references
/// inside the body (including component-prop checks on `<Component>`
/// nodes nested below the snippet).
///
/// Parameters are typed as `any` — we don't resolve their types from
/// the declared `Snippet<[...]>` shape here. For excess-prop and
/// "cannot find name" detection that's sufficient; precise param
/// typing would require threading the consumer's `Snippet<...>` into
/// the snippet body's type context, which v0.1 punts on.
///
/// Handles both identifier params (`foo, bar`) and destructure params
/// (`{months, weekdays}`, `[a, b]`) via `all_identifiers`. Default
/// values (`foo = 1`) have the default expression stripped before
/// identifier extraction.
pub(crate) fn emit_snippet_block(
    buf: &mut EmitBuffer,
    source: &str,
    b: &SnippetBlock,
    depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let indent = "    ".repeat(depth);
    let params_text = source
        .get(b.parameters_range.start as usize..b.parameters_range.end as usize)
        .unwrap_or("")
        .trim();
    // `{#snippet name()}` with an empty parameter list — skip the
    // scope + placeholder binding entirely. `all_identifiers` never
    // returns an empty vec (it falls back to `__svn_each_unused`),
    // so we intercept the empty-params case before calling it.
    if params_text.is_empty() {
        emit_template_body(buf, source, &b.body, depth, insts, action_counter);
        return;
    }
    // Open a fresh block, then reference the ORIGINAL params text
    // inside an arrow function's signature — that keeps the user's
    // type annotations (e.g. `MouseEventHandler<HTMLButtonElement>`)
    // bound as references. Without this, types only used in a
    // snippet parameter list (and never in the script body) fire
    // TS6133 "declared but never read" on their import.
    //
    // We don't try to thread contextual typing through — top-level
    // snippets have no parent satisfies target to flow from. Each
    // user-named parameter gets re-declared inside the body as `const
    // <name>: any` so the snippet body type-checks without depending
    // on the arrow's inferred param types (which the `null as any`
    // cast erases anyway).
    let annotated = annotate_snippet_params(params_text);
    let idents = all_identifiers(params_text);
    // Emit the body INSIDE a no-op arrow function expression whose
    // params carry the user's type annotations. This serves two
    // purposes simultaneously:
    //   1. The arrow params are the binding introductions — their
    //      type annotations flow into the body, so a snippet like
    //      `{#snippet state(state: VideoState)}` gets `state: VideoState`
    //      bindings rather than `state: any`.
    //   2. Any type imported solely for a snippet param annotation
    //      (e.g. `MouseEventHandler<HTMLButtonElement>`) shows up as
    //      a reference in the emitted code, suppressing TS6133.
    //
    // We use a function *expression* (not a type) so default values
    // and optional-after-required orderings remain legal.
    let _ = writeln!(buf, "{indent}{{");
    let _ = writeln!(buf, "{indent}    void (({annotated}) => {{");
    emit_template_body(buf, source, &b.body, depth + 2, insts, action_counter);
    for ident in &idents {
        let _ = writeln!(buf, "{indent}        void {ident};");
    }
    let _ = writeln!(buf, "{indent}    }});");
    let _ = writeln!(buf, "{indent}}}");
}
