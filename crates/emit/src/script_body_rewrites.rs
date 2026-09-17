//! Post-emit in-place rewrites of the instance script body.
//!
//! Passes that run after the script has been spliced into the emit
//! buffer, in order:
//!
//! 1. **Definite-assign the reactive targets** — `let X: T;` becomes
//!    `let X!: T;` for a name a `$:` statement assigns: the reactive
//!    rewrite wraps that statement in an uncalled arrow, so TS flow
//!    analysis never sees the assignment. (JS overlays get the
//!    JSDoc-cast initializer form instead.) A `bind:this` target is
//!    not one of them: upstream leaves its declaration as written, so
//!    a direct read before the element mounts still reports TS2454
//!    while reads inside closures pass.
//! 2. **Assert exported prop types** — upstream's
//!    `ExportedNames.propTypeAssertToUserDefined`: `;X = __svn_any(X);`
//!    after each exported `let` with no initializer, a declared type,
//!    or an untyped boolean initializer, so the prop keeps its declared
//!    (or widened) type instead of the initializer's literal type.
//!    A SvelteKit route's untyped `export const snapshot` gets its
//!    `Snapshot` type from `$types` right after.
//! 3. **Store subscriptions** — `;let $x = __svn_store_get(x);` after
//!    each declaration that binds a store read as `$x`.
//!
//! Every pass splices bytes into the buffer after the script body's
//! byte-precise token map was pushed, so each returns its insertions
//! and the token map is re-anchored immediately.

use std::ops::Range;
use std::path::Path;

use smol_str::SmolStr;

use crate::emit_buffer::EmitBuffer;
use crate::emit_is_ts;
use crate::process_instance_script_content;
use crate::store_subscriptions;
use crate::svelte4::compat::{
    annotate_exported_kit_consts_in_place, assert_exported_prop_types_in_place,
    rewrite_definite_assignment_in_place, widen_untyped_exports_jsdoc_in_place,
};
use crate::sveltekit;

/// Apply the post-body in-place rewrites (see the module doc for the
/// passes and their order).
pub(crate) fn apply_script_body_rewrites(
    buf: &mut EmitBuffer,
    split: Option<(&process_instance_script_content::SplitScript, Range<usize>)>,
    module_body: Option<Range<usize>>,
    mut store_bases: Vec<SmolStr>,
    reactive_touched_names: &[SmolStr],
    source_path: &Path,
) {
    // SVELTE-4-COMPAT: names touched by reactive destructure /
    // re-assignment. The reactive rewrite wraps block/expr-form `$:`
    // in an uncalled arrow so TS flow analysis misses the assignment.
    // No blanket source: injecting `!` on every typed uninitialised
    // `let` masked real TS2454 ("used before being assigned")
    // diagnostics that upstream surfaces unmodified.
    let mut def_assign_names: Vec<SmolStr> = Vec::new();
    for name in reactive_touched_names {
        if !def_assign_names.iter().any(|n| n == name) {
            def_assign_names.push(name.clone());
        }
    }
    let route_kind = sveltekit::route_kind(source_path);
    // The declaration rewrites only touch the spliced instance script.
    // Each pass grows the body by what it inserted, so the range is
    // re-extended before the next pass reads it.
    let apply = |buf: &mut EmitBuffer, body: &mut Range<usize>, edits: Vec<(u32, u32)>| {
        body.end += edits.iter().map(|&(_, len)| len as usize).sum::<usize>();
        buf.adjust_token_map_for_anchored_insertions(&edits);
    };
    // A store declared in the module script gets its `$store`
    // declaration there, at module scope, as upstream does. This runs
    // before the instance passes because the module text sits earlier
    // in the buffer, and those passes take the instance range as it is
    // after this insertion.
    let module_inserted: usize = if let Some(mut module) = module_body {
        let edits = store_subscriptions::attach_to_declarations(
            buf.raw_string_mut(),
            &module,
            &mut store_bases,
        );
        let inserted = edits.iter().map(|&(_, len)| len as usize).sum();
        apply(buf, &mut module, edits);
        inserted
    } else {
        0
    };
    let Some((s, mut body)) = split else {
        return;
    };
    body.start += module_inserted;
    body.end += module_inserted;
    if emit_is_ts() {
        let edits =
            rewrite_definite_assignment_in_place(buf.raw_string_mut(), &body, &def_assign_names);
        apply(buf, &mut body, edits);
    } else {
        // JS overlay: `let NAME;` → `let NAME = /** @type {any} */ (null);`
        // carries the definite-assign semantics in a JSDoc-only form
        // that survives `.svelte.svn.js` parsing without firing TS8010.
        let edits = widen_untyped_exports_jsdoc_in_place(
            buf.raw_string_mut(),
            &body,
            &def_assign_names,
            route_kind,
        );
        apply(buf, &mut body, edits);
    }
    let edits = assert_exported_prop_types_in_place(
        buf.raw_string_mut(),
        &body,
        &s.exported_locals,
        route_kind,
        emit_is_ts(),
    );
    apply(buf, &mut body, edits);
    let exported_consts: Vec<SmolStr> = s
        .export_type_infos
        .iter()
        .filter(|info| !info.is_let)
        .map(|info| info.name.clone())
        .collect();
    let edits = annotate_exported_kit_consts_in_place(
        buf.raw_string_mut(),
        &body,
        &exported_consts,
        route_kind,
        emit_is_ts(),
    );
    apply(buf, &mut body, edits);
    // Last, so the declarations land after every rewrite of the
    // statements they attach to.
    let edits =
        store_subscriptions::attach_to_declarations(buf.raw_string_mut(), &body, &mut store_bases);
    apply(buf, &mut body, edits);
}
