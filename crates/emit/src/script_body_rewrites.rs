//! Post-emit in-place rewrites of the instance script body.
//!
//! Three passes that run after the script has been spliced into the
//! emit buffer, in order:
//!
//! 1. **Widen untyped exports** — `export let foo;` becomes
//!    `let foo: any;` so an untyped Svelte-4 prop doesn't fire
//!    TS7034/7005.
//! 2. **Definite-assign** — `let X: T;` becomes `let X!: T;` for
//!    every name we know is assigned at runtime but TS flow
//!    analysis can't prove (exported props, store auto-subscribe
//!    bases, reactive-rewrite-touched names). A `bind:this` target
//!    is not one of them: upstream leaves its declaration as
//!    written, so a direct read before the element mounts still
//!    reports TS2454 while reads inside closures pass.
//! 3. **De-narrow typed literal inits** — `export let size: Size =
//!    'medium'` gets a `size = undefined as any;` trailer so later
//!    comparisons don't fire TS2367 ("no overlap"). TS-only.
//!
//! Order is load-bearing: widen inserts `: any` at the name
//! position, and definite-assign looks for a `:` annotation to
//! decide whether to add `!`. Running the passes in the other
//! order means `!` lands first and hides the original `:`
//! annotation from widen's scanner.
//!
//! For JS-overlay files all three TS-mode passes collapse into a
//! single `widen_untyped_exports_jsdoc_in_place` call that emits
//! JSDoc-cast initializers (`let NAME = /** @type {any} */ (null);`)
//! — both the type-widen and definite-assign semantics in a form
//! that survives `.svelte.svn.js` parsing without firing TS8010.

use std::ops::Range;
use std::path::Path;

use smol_str::SmolStr;

use crate::emit_buffer::EmitBuffer;
use crate::emit_is_ts;
use crate::process_instance_script_content;
use crate::store_subscriptions;
use crate::svelte4::compat::{
    denarrow_typed_exported_props_in_place, rewrite_definite_assignment_in_place,
    widen_untyped_exported_props_in_place, widen_untyped_exports_jsdoc_in_place,
};
use crate::sveltekit;
use svn_analyze::collect_typed_top_level_lets;

/// Apply the three post-body in-place rewrites: widen-untyped-exports →
/// definite-assign → de-narrow-typed-literal-inits.
///
/// Builds the `def_assign_names` set from three sources
/// (export-stripped locals, store-auto-subscribe bases,
/// reactive-rewrite-touched names) — all of which produce
/// declarations that Svelte treats as definitely-assigned at runtime
/// but TS flow analysis can't prove.
pub(crate) fn apply_script_body_rewrites<'alloc>(
    buf: &mut EmitBuffer,
    split: Option<(&process_instance_script_content::SplitScript, Range<usize>)>,
    module_body: Option<Range<usize>>,
    mut store_bases: Vec<SmolStr>,
    reactive_touched_names: &[SmolStr],
    parsed_instance: Option<&svn_parser::ParsedScript<'alloc>>,
    source_path: &Path,
) {
    let mut def_assign_names: Vec<SmolStr> = Vec::new();
    if let Some((s, _)) = &split {
        for name in &s.exported_locals {
            if !def_assign_names.iter().any(|n| n == name) {
                def_assign_names.push(name.clone());
            }
        }
    }
    // SVELTE-4-COMPAT: names touched by reactive destructure /
    // re-assignment. The reactive rewrite wraps block/expr-form `$:`
    // in an uncalled arrow so TS flow analysis misses the assignment.
    for name in reactive_touched_names {
        if !def_assign_names.iter().any(|n| n == name) {
            def_assign_names.push(name.clone());
        }
    }
    // No blanket source: injecting `!` on every typed uninitialised
    // `let` masked real TS2454 ("used before being assigned")
    // diagnostics that upstream surfaces unmodified. The three
    // explicit sources above cover every "Svelte assigns this at
    // runtime" case upstream also covers; everything else is a
    // user-source order bug that should fire TS2454.
    // Every pass below splices bytes into the buffer AFTER the script
    // body's byte-precise TokenMapEntry was pushed (and after the
    // module script's). Each pass therefore returns its insertions and
    // the token map is re-anchored immediately, so the map and the
    // buffer never disagree — otherwise every diagnostic later in the
    // script reverse-maps to the wrong column (or line, once a trailer
    // lands inside the mapped span).
    let route_kind = sveltekit::route_kind(source_path);
    // The declaration rewrites only touch the spliced instance script.
    // Each pass grows the body by what it inserted, so the range is
    // re-extended before the next pass reads it.
    let apply = |buf: &mut EmitBuffer, body: &mut Range<usize>, edits: Vec<(u32, u32)>| {
        body.end += edits.iter().map(|&(_, len)| len as usize).sum::<usize>();
        buf.adjust_token_map_for_insertions(&edits);
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
        {
            let edits = widen_untyped_exported_props_in_place(
                buf.raw_string_mut(),
                &body,
                &s.exported_locals,
                route_kind,
            );
            apply(buf, &mut body, edits);
        }
        let edits =
            rewrite_definite_assignment_in_place(buf.raw_string_mut(), &body, &def_assign_names);
        apply(buf, &mut body, edits);
    } else {
        // JS overlay: a single inline-initializer rewrite replaces
        // both TS-mode passes. `let NAME;` → `let NAME = /** @type
        // {any} */ (null);` carries both the type-widen (no TS7034/7005)
        // and definite-assign (no TS2454) semantics in a JSDoc-only
        // form — tsgo parses it cleanly under `.svelte.svn.js` without
        // firing TS8010.
        let edits = widen_untyped_exports_jsdoc_in_place(
            buf.raw_string_mut(),
            &body,
            &def_assign_names,
            route_kind,
        );
        apply(buf, &mut body, edits);
    }
    // SVELTE-4-COMPAT de-narrow: typed exported props with literal
    // initializers (`export let size: Size = 'medium'`) AND body-local
    // `let X: T = lit;` both narrow to the literal; inserting
    // `NAME = undefined as any;` after the declaration widens back to
    // the declared annotation, so later comparisons don't fire TS2367
    // "no overlap".
    //
    // TS-only: the inserted trailer uses `as any` which is TS syntax.
    // JS-overlay paths go through `widen_untyped_exports_jsdoc_in_place`
    // above, which emits the equivalent JSDoc-cast form that survives
    // `.svelte.svn.js` parsing without firing TS8010.
    if emit_is_ts() {
        let mut denarrow_targets: Vec<SmolStr> = s.exported_locals.clone();
        if let Some(parsed_orig) = parsed_instance {
            let mut typed_lets: Vec<SmolStr> = Vec::new();
            collect_typed_top_level_lets(&parsed_orig.program, &mut typed_lets);
            for name in typed_lets {
                if !denarrow_targets.iter().any(|n| n == &name) {
                    denarrow_targets.push(name);
                }
            }
        }
        let edits =
            denarrow_typed_exported_props_in_place(buf.raw_string_mut(), &body, &denarrow_targets);
        apply(buf, &mut body, edits);
    }
    // Last, so the declarations land after every rewrite of the
    // statements they attach to.
    let edits =
        store_subscriptions::attach_to_declarations(buf.raw_string_mut(), &body, &mut store_bases);
    apply(buf, &mut body, edits);
}
