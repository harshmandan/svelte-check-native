//! `{#snippet}` block emission.
//!
//! Mirrors upstream svelte2tsx's
//! `language-tools/packages/svelte2tsx/src/htmlxtojsx_v2/nodes/SnippetBlock.ts`.

use std::collections::HashMap;
use std::fmt::Write;

use svn_parser::SnippetBlock;

use crate::emit_buffer::EmitBuffer;
use crate::emit_template_body;
use crate::is_ts::emit_is_ts;

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
/// (`{months, weekdays}`, `[a, b]`) via `param_binding_names`. Default
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
    // Emit the same consolidated `const NAME = (params) => { … };
    // void NAME;` declaration the hoist path in `emit_template_body`
    // produces. The old shape here was a bare `{ void ((params) => {…}) }`
    // wrapper that never DECLARED `NAME`, so a sibling `{@render NAME()}`
    // fired a spurious TS2304. This path is reached when a snippet is
    // emitted directly via `emit_template_node` (e.g. from
    // `emit_children_with_let_bindings`) rather than through
    // `emit_template_body`'s snippet-collection hoist.
    emit_snippet_const(buf, source, b, depth, insts, action_counter);
}

/// Declared return type of a standalone snippet. A snippet is a
/// function whose result is Svelte's branded snippet-return value, so a
/// snippet handed to a prop that expects some other callback (say
/// `() => string`) is rejected, just as it is by Svelte's own types.
const SNIPPET_RETURN_TS: &str = ": ReturnType<import('svelte').Snippet>";
/// The same return type for a JavaScript overlay, where it can only be
/// stated in a JSDoc comment on the arrow.
const SNIPPET_RETURN_JSDOC: &str = "/** @returns {ReturnType<import('svelte').Snippet>} */ ";

/// Emit one `const NAME = (params): ReturnType<Snippet> => { <body>
/// … return __svn_any(0); }; void NAME;` snippet declaration at
/// `decl_depth`. Shared by both the `emit_template_body` hoist loop and `emit_snippet_block`, so
/// the snippet shape is single-sourced and matches upstream svelte2tsx's
/// `SnippetBlock.ts:117-140` (`const NAME = (params) => { … }`).
/// Write `const NAME`, mapping NAME to the snippet's name in the
/// source: svelte2tsx moves the name there, so a diagnostic on the
/// declaration (a script variable of the same name, TS2451) is
/// reported at `{#snippet NAME`.
fn write_snippet_const_head(buf: &mut EmitBuffer, source: &str, s: &SnippetBlock, decl: &str) {
    buf.push_str(decl);
    buf.push_str("const ");
    let head = source.get(s.range.start as usize..).unwrap_or("");
    let name_start = head
        .strip_prefix("{#snippet")
        .map(|rest| s.range.start + 9 + (rest.len() - rest.trim_start().len()) as u32)
        .filter(|&start| {
            source
                .get(start as usize..)
                .is_some_and(|t| t.starts_with(s.name.as_str()))
        });
    match name_start {
        Some(start) => buf.append_with_source(
            s.name.as_str(),
            svn_core::Range::new(start, start + s.name.len() as u32),
        ),
        None => buf.push_str(s.name.as_str()),
    }
}

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
    // `{#snippet row<T>(x: T)}` — splice the generic signature onto the
    // arrow so the type parameters bind at each `{@render}` call site.
    // Upstream emits `<${typeParams}>` only under TS syntax
    // (SnippetBlock.ts); an arrow generic is valid in a plain .ts
    // overlay (no .tsx ambiguity).
    let generics = if is_ts {
        s.generics_range
            .and_then(|r| source.get(r.start as usize..r.end as usize))
            .map(|g| format!("<{}>", g.trim()))
            .unwrap_or_default()
    } else {
        String::new()
    };
    // Empty-params snippet: skip the `(params)` site entirely so an
    // unused-arrow-param lint doesn't fire on a synthetic empty
    // signature, AND no identifier needs to be `void`'d.
    if params.is_empty() {
        // The body sits inside an inner `async () =>` wrapper —
        // upstream's "inner async function for potential #await
        // blocks" (`SnippetBlock.ts:70`) — so `{#await}` blocks emit
        // their `await` inline (see `await_pending_catch_block.rs`)
        // and still have an async context inside the sync snippet
        // arrow.
        write_snippet_const_head(buf, source, s, &decl);
        if is_ts {
            let _ = writeln!(
                buf,
                " = {generics}(){SNIPPET_RETURN_TS} => {{ async () => {{"
            );
        } else {
            let _ = writeln!(buf, " = {SNIPPET_RETURN_JSDOC}() => {{ async () => {{");
        }
        emit_template_body(buf, source, &s.body, body_depth, insts, action_counter);
        let _ = writeln!(buf, "{body_i}}};");
        let _ = writeln!(buf, "{body_i}return __svn_any(0);");
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
    let raw = source
        .get(s.parameters_range.start as usize..s.parameters_range.end as usize)
        .unwrap_or("");
    let leading_ws = (raw.len() - raw.trim_start().len()) as u32;
    let params_start = s.parameters_range.start + leading_ws;
    let params_range = svn_core::Range::new(params_start, params_start + params.len() as u32);
    let jsdoc = if is_ts { "" } else { SNIPPET_RETURN_JSDOC };
    write_snippet_const_head(buf, source, s, &decl);
    let _ = write!(buf, " = {jsdoc}{generics}(");
    buf.append_with_source(params, params_range);
    // Inner `async () =>` wrapper: upstream's "inner async function
    // for potential #await blocks" (`SnippetBlock.ts:70`), giving
    // inline-emitted `await`s an async context inside the sync
    // snippet arrow. The param `void`s and the return stay in the
    // OUTER arrow, whose scope the params belong to.
    if is_ts {
        let _ = writeln!(buf, "){SNIPPET_RETURN_TS} => {{ async () => {{");
    } else {
        let _ = writeln!(buf, ") => {{ async () => {{");
    }
    emit_template_body(buf, source, &s.body, body_depth, insts, action_counter);
    let _ = writeln!(buf, "{body_i}}};");
    let _ = writeln!(buf, "{body_i}return __svn_any(0);");
    let _ = writeln!(buf, "{decl}}};");
    let _ = writeln!(buf, "{decl}void {};", s.name);
}
