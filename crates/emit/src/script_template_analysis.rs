//! Cross-cutting analyze pass that runs after hoisted imports but
//! before the template-check wrapper.
//!
//! Produces five named buckets in [`ScriptAndTemplateAnalysis`]:
//!
//! - `prop_names` — destructured names from `let { … } = $props()`.
//! - `prop_type_source` — the Props type text emit will use, with
//!   the SvelteKit route-prop synth (PageData / LayoutData /
//!   ActionData) folded in when a route file has no user-provided
//!   source.
//! - `store_refs` — `$store` auto-subscribe references from both
//!   script sides AND the template, deduplicated, in encounter
//!   order.
//! - `template_void_refs` — script bindings used only in markup.
//!   The emit's `void(...)` block keeps these alive so TS6133
//!   doesn't fire on what's actually used.
//! - `template_type_refs` — type-only imports referenced from
//!   template type-cast expressions; emit registers them in a
//!   module-scope `type __svn_tpl_type_refs = [A]` so the imports
//!   stay visible to the type-checker.
//!
//! Pulled out of `lib.rs` so the dispatcher reads as orchestration
//! and this 150-line analyze concern lives with its data.

use std::collections::HashSet;
use std::path::Path;

use oxc_allocator::Allocator;
use smol_str::SmolStr;
use svn_analyze::{
    PropsInfo, collect_top_level_bindings, find_store_refs_with_bindings, find_template_refs,
};
use svn_parser::parse_script_body;

use crate::process_instance_script_content;
use crate::sveltekit;

/// Props, store auto-subscribes, and template-referenced identifier
/// buckets — see module docs.
pub(crate) struct ScriptAndTemplateAnalysis {
    pub prop_names: Vec<SmolStr>,
    pub prop_type_source: Option<String>,
    pub store_refs: Vec<SmolStr>,
    pub template_void_refs: Vec<SmolStr>,
    pub template_type_refs: Vec<SmolStr>,
}

/// Run the cross-cutting analyze pass — see module docs for the
/// shape produced.
///
/// Script-binding collection unions the module script, the instance
/// script (original, with imports visible), and the rewritten
/// content (so reactive-destructure-introduced names — `$: ({a, b}
/// = expr)` → `let {a, b} = …` — participate in subsequent `$a`/`$b`
/// store-alias detection).
#[allow(clippy::too_many_arguments)]
pub(crate) fn analyze_script_and_template_refs<'alloc>(
    doc: &svn_parser::Document<'_>,
    source_path: &Path,
    fragment: &svn_parser::Fragment,
    parsed_instance: Option<&svn_parser::ParsedScript<'alloc>>,
    split: Option<&process_instance_script_content::SplitScript>,
    rewritten_content: Option<&str>,
    props_info: &PropsInfo,
    effective_props_type_text: Option<&str>,
) -> ScriptAndTemplateAnalysis {
    // Parse the module script once up front; both `script_bindings`
    // collection and the type-only-import scan below consume it.
    // Allocator lives at function scope so the AST stays valid across
    // both consumers.
    let alloc_mod = Allocator::default();
    let parsed_mod = doc
        .module_script
        .as_ref()
        .map(|ms| parse_script_body(&alloc_mod, ms.content, ms.lang));

    let mut script_bindings: HashSet<String> = HashSet::new();
    if let Some(parsed) = &parsed_mod {
        collect_top_level_bindings(&parsed.program, &mut script_bindings);
    }

    let (prop_names, prop_type_source): (Vec<SmolStr>, Option<String>) =
        if let (Some(_s), Some(instance), Some(parsed_orig)) =
            (split, &doc.instance_script, parsed_instance)
        {
            let props: Vec<SmolStr> = props_info
                .destructures
                .iter()
                .map(|p| p.local_name.clone())
                .collect();

            // SvelteKit auto-typing: route components (+page.svelte,
            // +layout.svelte) with an untyped `$props()` pick up
            // `PageData` / `LayoutData` / `ActionData` from the file
            // path + the list of destructured prop names. Only fires
            // when PropsInfo saw no user-provided source.
            let ty = effective_props_type_text.map(|s| s.to_string()).or_else(|| {
                sveltekit::route_kind(source_path).and_then(|kind| {
                    let names_borrow: Vec<&str> = props.iter().map(|s| s.as_str()).collect();
                    sveltekit::synthesize_route_props_type(kind, &names_borrow)
                })
            });

            collect_top_level_bindings(&parsed_orig.program, &mut script_bindings);
            if let Some(rewritten) = rewritten_content {
                let alloc_rw = Allocator::default();
                let parsed_rw = parse_script_body(&alloc_rw, rewritten, instance.lang);
                collect_top_level_bindings(&parsed_rw.program, &mut script_bindings);
            }
            (props, ty)
        } else {
            (Vec::new(), None)
        };

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
        if let Some(module_script) = &doc.module_script {
            push_unique(
                find_store_refs_with_bindings(module_script.content, &script_bindings),
                &mut seen,
                &mut accumulated,
            );
        }
        if let Some(instance) = &doc.instance_script {
            push_unique(
                find_store_refs_with_bindings(instance.content, &script_bindings),
                &mut seen,
                &mut accumulated,
            );
        }
        accumulated
    };

    // Type-only imports can be "used" purely inside a template
    // expression (type cast `{foo(item as AppVideo)}`); we intersect
    // with template refs below and emit `type __svn_tpl_type_refs = [A]`
    // so TS doesn't flag the import TS6133.
    let mut type_only_imports: HashSet<String> = HashSet::new();
    if let Some(parsed) = parsed_instance {
        svn_analyze::collect_type_only_import_bindings(&parsed.program, &mut type_only_imports);
    }
    if let Some(parsed) = &parsed_mod {
        svn_analyze::collect_type_only_import_bindings(&parsed.program, &mut type_only_imports);
    }

    // Single template walk produces: void-refs (script bindings used
    // only in markup), template-side store-auto-subscribes, and type-
    // only-import type refs.
    let (template_void_refs, template_store_refs, template_type_refs) = if script_bindings
        .is_empty()
        && type_only_imports.is_empty()
    {
        (Vec::new(), Vec::new(), Vec::new())
    } else {
        let already: HashSet<&str> = store_refs
            .iter()
            .chain(prop_names.iter())
            .map(|s| s.as_str())
            .collect();
        let mut tpl_voids = Vec::new();
        let mut tpl_stores = Vec::new();
        let mut tpl_types: Vec<SmolStr> = Vec::new();
        let mut type_seen: HashSet<SmolStr> = HashSet::new();
        let mut store_seen: HashSet<SmolStr> = store_refs.iter().cloned().collect();
        for name in find_template_refs(fragment, doc.source) {
            if let Some(base) = name.as_str().strip_prefix('$') {
                if script_bindings.contains(base) && store_seen.insert(name.clone()) {
                    tpl_stores.push(name.clone());
                    continue;
                }
            }
            if script_bindings.contains(name.as_str()) && !already.contains(name.as_str()) {
                tpl_voids.push(name);
            } else if type_only_imports.contains(name.as_str()) && type_seen.insert(name.clone()) {
                tpl_types.push(name);
            }
        }
        (tpl_voids, tpl_stores, tpl_types)
    };
    store_refs.extend(template_store_refs);

    ScriptAndTemplateAnalysis {
        prop_names,
        prop_type_source,
        store_refs,
        template_void_refs,
        template_type_refs,
    }
}
