//! `{#snippet}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/SnippetBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::SnippetBlock;

use crate::emit_buffer::EmitBuffer;
use crate::is_ts::emit_is_ts;
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
    // Emit the same consolidated `const NAME = (params): any => { … };
    // void NAME;` declaration the hoist path in `emit_template_body`
    // produces. The old shape here was a bare `{ void ((params) => {…}) }`
    // wrapper that never DECLARED `NAME`, so a sibling `{@render NAME()}`
    // fired a spurious TS2304. This path is reached when a snippet is
    // emitted directly via `emit_template_node` (e.g. from
    // `emit_children_with_let_bindings`) rather than through
    // `emit_template_body`'s snippet-collection hoist.
    emit_snippet_const(buf, source, b, depth, insts, action_counter);
}

/// Emit one `const NAME = (params): any => { <body> … return null as
/// any; }; void NAME;` snippet declaration at `decl_depth`. Shared by
/// both the `emit_template_body` hoist loop and `emit_snippet_block`, so
/// the snippet shape is single-sourced and matches upstream svelte2tsx's
/// `SnippetBlock.ts:117-140` (`const NAME = (params) => { … }`).
pub(crate) fn emit_snippet_const(
    buf: &mut EmitBuffer,
    source: &str,
    s: &SnippetBlock,
    decl_depth: usize,
    insts: &HashMap<u32, &svn_analyze::ComponentInstantiation>,
    action_counter: &mut usize,
) {
    let is_ts = emit_is_ts();
    let decl = "    ".repeat(decl_depth);
    let body_depth = decl_depth + 1;
    let body_i = "    ".repeat(body_depth);
    let params = source
        .get(s.parameters_range.start as usize..s.parameters_range.end as usize)
        .unwrap_or("")
        .trim();
    // Empty-params snippet: skip the `(params)` site entirely so an
    // unused-arrow-param lint doesn't fire on a synthetic empty
    // signature, AND no identifier needs to be `void`'d.
    if params.is_empty() {
        if is_ts {
            let _ = writeln!(buf, "{decl}const {} = (): any => {{", s.name);
        } else {
            let _ = writeln!(buf, "{decl}const {} = () => {{", s.name);
        }
        emit_template_body(buf, source, &s.body, body_depth, insts, action_counter);
        if is_ts {
            let _ = writeln!(buf, "{body_i}return null as any;");
        } else {
            let _ = writeln!(buf, "{body_i}return null;");
        }
        let _ = writeln!(buf, "{decl}}};");
        let _ = writeln!(buf, "{decl}void {};", s.name);
        return;
    }
    // The arrow params are the binding introductions — their type
    // annotations flow into the body (e.g. `{#snippet row(v: VideoState)}`
    // gets `v: VideoState`), and any type imported solely for a snippet
    // param annotation shows up as a reference, suppressing TS6133. For
    // TS overlays append `: any` to each unannotated param so the body
    // type-checks under `--strict` without `noImplicitAny` firing; for JS
    // overlays keep params verbatim so JSDoc / default-value inference
    // still flows.
    let annotated = if is_ts {
        annotate_snippet_params(params)
    } else {
        params.to_string()
    };
    let idents = all_identifiers(params);
    if is_ts {
        let _ = writeln!(buf, "{decl}const {} = ({annotated}): any => {{", s.name);
    } else {
        let _ = writeln!(buf, "{decl}const {} = ({annotated}) => {{", s.name);
    }
    emit_template_body(buf, source, &s.body, body_depth, insts, action_counter);
    for ident in &idents {
        let _ = writeln!(buf, "{body_i}void {ident};");
    }
    if is_ts {
        let _ = writeln!(buf, "{body_i}return null as any;");
    } else {
        let _ = writeln!(buf, "{body_i}return null;");
    }
    let _ = writeln!(buf, "{decl}}};");
    let _ = writeln!(buf, "{decl}void {};", s.name);
}
