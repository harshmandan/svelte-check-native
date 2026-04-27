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
//!    analysis can't prove (bind:this targets, store
//!    auto-subscribe bases, reactive-rewrite-touched names, typed
//!    uninitialised top-level lets).
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

use std::path::Path;

use smol_str::SmolStr;

use crate::emit_buffer::EmitBuffer;
use crate::emit_is_ts;
use crate::process_instance_script_content;
use crate::svelte4::compat::{
    denarrow_typed_exported_props_in_place, rewrite_definite_assignment_in_place,
    widen_untyped_exported_props_in_place, widen_untyped_exports_jsdoc_in_place,
};
use crate::sveltekit;
use svn_analyze::{collect_typed_top_level_lets, collect_typed_uninit_lets};

/// Apply the three post-body in-place rewrites: widen-untyped-exports →
/// definite-assign → de-narrow-typed-literal-inits.
///
/// Builds the `def_assign_names` set from five sources (bind:this
/// targets, export-stripped locals, store-auto-subscribe bases,
/// reactive-rewrite-touched names, typed uninitialized top-level
/// `let`s) — all of which produce declarations that Svelte treats
/// as definitely-assigned at runtime but TS flow analysis can't
/// prove.
pub(crate) fn apply_script_body_rewrites<'alloc>(
    buf: &mut EmitBuffer,
    summary: &svn_analyze::TemplateSummary,
    split: Option<&process_instance_script_content::SplitScript>,
    store_refs: &[SmolStr],
    reactive_touched_names: &[SmolStr],
    parsed_instance: Option<&svn_parser::ParsedScript<'alloc>>,
    source_path: &Path,
) {
    let mut def_assign_names: Vec<SmolStr> = summary
        .bind_this_targets
        .iter()
        .map(|t| t.name.clone())
        .collect();
    if let Some(s) = split {
        for name in &s.exported_locals {
            if !def_assign_names.iter().any(|n| n == name) {
                def_assign_names.push(name.clone());
            }
        }
    }
    // `$store` auto-subscribe aliases: definite-assign the underlying
    // `store` local. Body-declared `let store: Writable<T>` without
    // initializer fires TS2454 at every `typeof store` read; the
    // rewrite is a no-op for imports / `const` / initialized `let`.
    for name in store_refs {
        let base = SmolStr::from(name.strip_prefix('$').unwrap_or(name));
        if !def_assign_names.iter().any(|n| n == &base) {
            def_assign_names.push(base);
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
    // Every top-level `let NAME: Type;` (typed, no init) in the
    // instance script — the Svelte "declare now, assign later from a
    // handler" pattern. Upstream's TS version doesn't observe the
    // uninitialised state across its transform pipeline; a `!` gives
    // us the same behavior.
    if let Some(parsed_orig) = parsed_instance {
        let mut uninit_lets: Vec<SmolStr> = Vec::new();
        collect_typed_uninit_lets(&parsed_orig.program, &mut uninit_lets);
        for name in uninit_lets {
            if !def_assign_names.iter().any(|n| n == &name) {
                def_assign_names.push(name);
            }
        }
    }
    let route_kind = sveltekit::route_kind(source_path);
    if emit_is_ts() {
        if let Some(s) = split {
            widen_untyped_exported_props_in_place(
                buf.raw_string_mut(),
                &s.exported_locals,
                route_kind,
            );
        }
        rewrite_definite_assignment_in_place(buf.raw_string_mut(), &def_assign_names);
    } else {
        // JS overlay: a single inline-initializer rewrite replaces
        // both TS-mode passes. `let NAME;` → `let NAME = /** @type
        // {any} */ (null);` carries both the type-widen (no TS7034/7005)
        // and definite-assign (no TS2454) semantics in a JSDoc-only
        // form — tsgo parses it cleanly under `.svelte.svn.js` without
        // firing TS8010.
        widen_untyped_exports_jsdoc_in_place(buf.raw_string_mut(), &def_assign_names, route_kind);
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
    if emit_is_ts()
        && let Some(s) = split
    {
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
        denarrow_typed_exported_props_in_place(buf.raw_string_mut(), &denarrow_targets);
    }
}
