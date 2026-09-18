//! Cross-cutting analyze pass that runs after hoisted imports but
//! before the template-check wrapper.
//!
//! Produces three named buckets in [`ScriptAndTemplateAnalysis`]:
//!
//! - `bindable_prop_names` — the destructured `let { … } = $props()`
//!   names whose entry is `= $bindable(...)`. Only these get the
//!   outer-scope `void <name>;` emission — non-bindable prop locals
//!   are left exposed so TS6133 fires on the ones that are never
//!   read (matching upstream svelte2tsx's
//!   `ExportedNames.ts:197-204` `;foo;`-on-$bindable behaviour).
//! - `prop_type_source` — the Props type text emit will use, with
//!   the SvelteKit route-prop synth (PageData / LayoutData /
//!   ActionData) folded in when a route file has no user-provided
//!   source.
//! - `store_refs` — `$store` auto-subscribe references from both
//!   script sides AND the template, deduplicated, in encounter
//!   order.
//!
//! Template reads need no bookkeeping of their own: every template
//! expression is copied into the render function, so TypeScript sees
//! each read where it happens, exactly as upstream does.
//!
//! Pulled out of `lib.rs` so the dispatcher reads as orchestration
//! and this 150-line analyze concern lives with its data.

use std::collections::HashSet;

use oxc_allocator::Allocator;
use smol_str::SmolStr;
use svn_analyze::{
    PropsInfo, collect_rune_scan_context, collect_top_level_bindings,
    find_store_refs_with_bindings, find_template_store_refs, has_svelte_store_derived_import,
};
use svn_parser::parse_script_body;

use crate::process_instance_script_content;

/// Props, store auto-subscribes, and template-referenced identifier
/// buckets — see module docs.
pub(crate) struct ScriptAndTemplateAnalysis {
    pub bindable_prop_names: Vec<SmolStr>,
    pub prop_type_source: Option<String>,
    pub store_refs: Vec<SmolStr>,
}

/// Run the cross-cutting analyze pass — see module docs for the
/// shape produced. `store_refs` comes from [`collect_store_refs`],
/// which runs earlier because the script split needs it.
pub(crate) fn analyze_script_and_template_refs<'alloc>(
    doc: &svn_parser::Document<'_>,
    parsed_instance: Option<&svn_parser::ParsedScript<'alloc>>,
    split: Option<&process_instance_script_content::SplitScript>,
    props_info: &PropsInfo,
    effective_props_type_text: Option<&str>,
    store_refs: Vec<SmolStr>,
) -> ScriptAndTemplateAnalysis {
    // `local_only` leaves are excluded: upstream's `;prop;`-on-
    // $bindable emission only fires for simple top-level elements.
    let bindable_prop_names: Vec<SmolStr> = props_info
        .destructures
        .iter()
        .filter(|p| p.is_bindable && !p.local_only)
        .map(|p| p.local_name.clone())
        .collect();

    let prop_type_source: Option<String> = match (split, &doc.instance_script, parsed_instance) {
        (Some(_), Some(_), Some(_)) => effective_props_type_text.map(|s| s.to_string()),
        _ => None,
    };

    ScriptAndTemplateAnalysis {
        bindable_prop_names,
        prop_type_source,
        store_refs,
    }
}

/// `$store` auto-subscribe references from both script sides and the
/// template, deduplicated, in encounter order.
///
/// Script-binding collection unions the module script, the instance
/// script (original, with imports visible), and the rewritten
/// content (so reactive-destructure-introduced names — `$: ({a, b}
/// = expr)` → `let {a, b} = …` — participate in subsequent `$a`/`$b`
/// store-alias detection).
pub(crate) fn collect_store_refs<'alloc>(
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
    parsed_instance: Option<&svn_parser::ParsedScript<'alloc>>,
    rewritten_content: Option<&str>,
) -> Vec<SmolStr> {
    let alloc_mod = Allocator::default();
    let parsed_mod = doc
        .module_script
        .as_ref()
        .map(|ms| parse_script_body(&alloc_mod, ms.content, ms.lang));

    let mut script_bindings: HashSet<String> = HashSet::new();
    // Type-only imports count too: upstream declares `$name` for any
    // imported `name` it sees read that way (inside ignore comments,
    // so a type that isn't a store just leaves `$name` as `any`).
    let mut imports: HashSet<SmolStr> = HashSet::new();
    if let Some(parsed) = &parsed_mod {
        collect_top_level_bindings(&parsed.program, &mut script_bindings);
        crate::store_subscriptions::import_local_names(&parsed.program, &mut imports);
    }
    if let (Some(instance), Some(parsed_orig)) = (&doc.instance_script, parsed_instance) {
        collect_top_level_bindings(&parsed_orig.program, &mut script_bindings);
        crate::store_subscriptions::import_local_names(&parsed_orig.program, &mut imports);
        if let Some(rewritten) = rewritten_content {
            let alloc_rw = Allocator::default();
            let parsed_rw = parse_script_body(&alloc_rw, rewritten, instance.lang);
            collect_top_level_bindings(&parsed_rw.program, &mut script_bindings);
        }
    }
    script_bindings.extend(imports.into_iter().map(String::from));

    // Store auto-subscribe scan happens AFTER both module + instance
    // bindings are collected, so a `$properties` use in instance can
    // resolve to a `properties` declared in `<script module>`.
    let mut store_refs: Vec<SmolStr> = {
        let mut accumulated: Vec<SmolStr> = Vec::new();
        let mut seen: HashSet<SmolStr> = HashSet::new();
        let push_unique =
            |found: Vec<SmolStr>, seen: &mut HashSet<SmolStr>, out: &mut Vec<SmolStr>| {
                for name in found {
                    if seen.insert(name.clone()) {
                        out.push(name);
                    }
                }
            };
        if let (Some(module_script), Some(p)) = (&doc.module_script, parsed_mod.as_ref()) {
            // Rune-position skips are per-script — the offsets index
            // into the script content being walked.
            let runes = collect_rune_scan_context(&p.program, module_script.content);
            push_unique(
                find_store_refs_with_bindings(
                    &p.program,
                    module_script.content,
                    &script_bindings,
                    &runes,
                ),
                &mut seen,
                &mut accumulated,
            );
        }
        if let (Some(instance), Some(p)) = (&doc.instance_script, parsed_instance) {
            let runes = collect_rune_scan_context(&p.program, instance.content);
            push_unique(
                find_store_refs_with_bindings(
                    &p.program,
                    instance.content,
                    &script_bindings,
                    &runes,
                ),
                &mut seen,
                &mut accumulated,
            );
        }
        accumulated
    };

    // Store subscriptions written in the template (`{$count}`): the
    // alias must exist for them just like for script-side reads.
    let template_store_refs: Vec<SmolStr> =
        find_template_store_refs(fragment, doc.source, &script_bindings)
            .into_iter()
            .filter(|name| !store_refs.contains(name))
            .collect();
    store_refs.extend(template_store_refs);

    // Mirrors upstream ImplicitStoreValues.isSvelteStoreDerivedImport
    // (Svelte 5+): a named import of `derived` from 'svelte/store'
    // never gets a store subscription — `$derived(...)` anywhere in
    // the component (script OR template) stays the rune.
    let derived_import = parsed_instance
        .map(|p| has_svelte_store_derived_import(&p.program))
        .unwrap_or(false)
        || parsed_mod
            .as_ref()
            .map(|p| has_svelte_store_derived_import(&p.program))
            .unwrap_or(false);
    if derived_import {
        store_refs.retain(|r| r != "$derived");
    }

    store_refs
}
