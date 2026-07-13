//! `{#snippet}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/SnippetBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::SnippetBlock;

use crate::emit_buffer::EmitBuffer;
use crate::is_ts::emit_is_ts;
use crate::{all_identifiers, emit_template_body};

/// Emit a lexical-scope block wrapping a `{#snippet name(params)}` body
/// so the snippet's parameter identifiers are in scope for references
/// inside the body (including component-prop checks on `<Component>`
/// nodes nested below the snippet).
///
/// Parameters are spliced VERBATIM with a token-map anchor, exactly
/// like upstream (`SnippetBlock.ts` moves the param list as-is with
/// source mapping). An unannotated param therefore fires TS7006 under
/// `noImplicitAny` just as upstream does, and any diagnostic landing
/// in the param list maps back to the .svelte source instead of being
/// dropped.
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
    // param annotation shows up as a reference, suppressing TS6133.
    // Splice them VERBATIM with a token-map anchor: upstream moves the
    // param list as-is with source mapping (`SnippetBlock.ts`), so an
    // unannotated param fires TS7006 under `noImplicitAny` exactly like
    // upstream, and diagnostics landing in the param list map back to
    // the user's source instead of being dropped.
    let idents = all_identifiers(params);
    let raw = source
        .get(s.parameters_range.start as usize..s.parameters_range.end as usize)
        .unwrap_or("");
    let leading_ws = (raw.len() - raw.trim_start().len()) as u32;
    let params_start = s.parameters_range.start + leading_ws;
    let params_range = svn_core::Range::new(params_start, params_start + params.len() as u32);
    let _ = write!(buf, "{decl}const {} = (", s.name);
    buf.append_with_source(params, params_range);
    if is_ts {
        let _ = writeln!(buf, "): any => {{");
    } else {
        let _ = writeln!(buf, ") => {{");
    }
    emit_template_body(buf, source, &s.body, body_depth, insts, action_counter);
    for ident in &idents {
        if *ident == "__svn_each_unused" {
            continue;
        }
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
