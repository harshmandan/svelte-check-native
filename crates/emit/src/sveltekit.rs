//! SvelteKit route-file detection and prop auto-typing.
//!
//! When the user writes a route component like:
//!
//! ```svelte
//! <script lang="ts">
//!     let { data } = $props();
//! </script>
//! {data.title}
//! ```
//!
//! `data` is a SvelteKit-injected prop with a known shape — `PageData` for
//! `+page.svelte`, `LayoutData` for `+layout.svelte`, etc. Upstream's
//! svelte2tsx synthesizes a destructure type annotation pointing at
//! `import('./$types.js').PageData` so the user's body reads a properly
//! typed `data`, not `any`.
//!
//! We do the same, but one layer up: when the user's `$props()` call has
//! NO type annotation and the file's basename is a route pattern, we
//! synthesize an inline object type from the destructured prop names and
//! feed it to the existing prop_type_source pipeline. The default export
//! then becomes `Component<{data: PageData, ...}>` and contextual typing
//! flows in the usual way.
//!
//! ### Scope: .svelte files only
//!
//! Upstream's `upsertKitFile` in `svelte2tsx/src/helpers/sveltekit.ts`
//! ALSO injects types into raw route `.ts` files (`+page.ts`,
//! `+page.server.ts`, `+server.ts`): it adds `: boolean | 'auto'` to
//! `prerender`, typed `RequestEvent` params to `GET`/`POST`/etc. Those
//! are not `.svelte` — our pipeline hands them to tsgo as user-owned
//! `.ts` and never produces an overlay for them. Matching upstream on
//! raw `.ts` files would require a separate mechanism (ambient decl or
//! tsconfig augmentation) that's out of scope for this crate. This
//! module handles `.svelte` route files only; consumers writing
//! `+page.ts` etc. rely on their own `$types.d.ts` imports resolving
//! through the user's tsconfig paths.

use std::path::Path;

use svn_core::sveltekit::{KitFilesSettings, KitRole, classify};

/// The kind of SvelteKit `.svelte` route component a basename matches.
///
/// Re-exported from the centralised `svn_core::sveltekit::RouteShape`
/// — emit/lib.rs has many `RouteKind::Page` / `Layout` / `Error`
/// callsites that keep working through the alias without a churning
/// rename. See `notes/PLAN-sveltekit-path-centralization.md` (Phase 4).
pub type RouteKind = svn_core::sveltekit::RouteShape;

/// Inspect `path` and return a `RouteKind` when its basename matches
/// a SvelteKit `.svelte` route component (`+page` / `+layout` /
/// `+error`, with optional `@group` suffix), or `None` otherwise.
///
/// `.ts` / `.js` route shapes are out of scope here — they go through
/// `kit_inject` instead. Filtering on `KitRole::RouteComponent` at
/// the centralised classifier picks exactly the `.svelte` set.
pub fn route_kind(path: &Path) -> Option<RouteKind> {
    let kit = classify(path, &KitFilesSettings::default())?;
    match kit.role {
        // `@group` layout-breakout suffixes exist for `+page` /
        // `+layout` only — SvelteKit has no `+error@x.svelte`. The
        // centralised classifier still parses the group label off any
        // route component, but upstream's `isKitErrorFile` requires
        // the stem to be exactly `+error`, so a grouped error basename
        // is NOT a Kit error file and gets no auto-typed props.
        KitRole::RouteComponent { shape } => match shape {
            RouteKind::Error if kit.group.is_some() => None,
            shape => Some(shape),
        },
        _ => None,
    }
}

/// Return the property declaration a SvelteKit route component's
/// `$props()` destructure key contributes to the synthesised props
/// type, or `None` when the key contributes nothing.
///
/// Mirrors upstream svelte2tsx's `handle$propsRune`: on a route file
/// ONLY the props SvelteKit itself passes are typed — `data`, `form`
/// (pages only; a layout's `$types` has no `ActionData`) and `params`
/// — and every other destructured key is left out of the type, so
/// reading it off the typed destructure is an error. An error page
/// types only `error`, from the app-wide `App.Error` ambient.
pub fn kit_prop_decl(name: &str, kind: RouteKind) -> Option<&'static str> {
    match (kind, name) {
        (RouteKind::Page, "data") => Some("data: import('./$types.js').PageData"),
        (RouteKind::Page, "form") => Some("form: import('./$types.js').ActionData"),
        (RouteKind::Page, "params") => Some("params: import('./$types.js').PageProps['params']"),
        (RouteKind::Layout, "data") => Some("data: import('./$types.js').LayoutData"),
        (RouteKind::Layout, "params") => {
            Some("params: import('./$types.js').LayoutProps['params']")
        }
        (RouteKind::Error, "error") => Some("error: App.Error"),
        _ => None,
    }
}

/// Build the props type for a SvelteKit route component whose
/// `$props()` destructure carries no type of its own, or `None` when
/// the component's props end up untyped.
///
/// Mirrors upstream svelte2tsx's best-effort synthesis on route files:
/// each simple destructure key contributes its [`kit_prop_decl`], a
/// layout always gains a required `children` snippet (SvelteKit always
/// renders one into it), and a pattern with non-simple elements
/// (`...rest`, nested patterns, computed keys) widens the result with
/// `& Record<string, any>` — or is bare `Record<string, any>` when
/// nothing else was pushed. With nothing pushed and nothing to widen,
/// upstream declares no props type at all and the component's props
/// resolve to `any`; `None` tells the caller to do the same.
pub fn synthesize_route_props_type(
    kind: RouteKind,
    prop_keys: &[&str],
    with_unknown: bool,
) -> Option<String> {
    let mut props: Vec<&str> = prop_keys
        .iter()
        .filter_map(|key| kit_prop_decl(key, kind))
        .collect();
    if matches!(kind, RouteKind::Layout) {
        props.push("children: import('svelte').Snippet");
    }
    if props.is_empty() {
        return with_unknown.then(|| "Record<string, any>".to_string());
    }
    let mut out = String::with_capacity(props.iter().map(|p| p.len() + 2).sum::<usize>() + 32);
    out.push_str("{ ");
    out.push_str(&props.join(", "));
    out.push_str(" }");
    if with_unknown {
        out.push_str(" & Record<string, any>");
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Direct `route_kind` tests (basename matrix, `@group` stripping,
    // `.ts`/`.js` rejection) live in `svn_core::sveltekit::tests` —
    // the centralised classifier exercises every shape there. Tests
    // below cover the emit-specific surface that reads `RouteKind`.

    #[test]
    fn route_kind_rejects_grouped_error_basename() {
        // `+error@x.svelte` isn't a Kit file (no `@` breakout for
        // error routes) — upstream's `isKitErrorFile` requires the
        // stem be exactly `+error`. Grouped pages/layouts stay
        // recognized.
        assert_eq!(route_kind(Path::new("src/routes/+error@x.svelte")), None);
        assert_eq!(
            route_kind(Path::new("src/routes/+error.svelte")),
            Some(RouteKind::Error)
        );
        assert_eq!(
            route_kind(Path::new("src/routes/(auth)/+page@(app).svelte")),
            Some(RouteKind::Page)
        );
    }

    #[test]
    fn kit_prop_decl_types_only_kit_passed_props() {
        assert_eq!(
            kit_prop_decl("params", RouteKind::Page),
            Some("params: import('./$types.js').PageProps['params']")
        );
        assert_eq!(
            kit_prop_decl("params", RouteKind::Layout),
            Some("params: import('./$types.js').LayoutProps['params']")
        );
        assert_eq!(kit_prop_decl("form", RouteKind::Layout), None);
        assert_eq!(kit_prop_decl("children", RouteKind::Layout), None);
        assert_eq!(kit_prop_decl("heading", RouteKind::Page), None);
        assert_eq!(kit_prop_decl("data", RouteKind::Error), None);
    }

    #[test]
    fn synth_page_drops_user_props() {
        let ty = synthesize_route_props_type(RouteKind::Page, &["data", "form", "heading"], false)
            .unwrap();
        assert_eq!(
            ty,
            "{ data: import('./$types.js').PageData, form: import('./$types.js').ActionData }"
        );
    }

    #[test]
    fn synth_layout_always_adds_children() {
        let ty =
            synthesize_route_props_type(RouteKind::Layout, &["data", "children"], false).unwrap();
        assert_eq!(
            ty,
            "{ data: import('./$types.js').LayoutData, children: import('svelte').Snippet }"
        );
        assert_eq!(
            synthesize_route_props_type(RouteKind::Layout, &[], false).as_deref(),
            Some("{ children: import('svelte').Snippet }")
        );
    }

    #[test]
    fn synth_widens_for_non_simple_elements() {
        assert_eq!(
            synthesize_route_props_type(RouteKind::Page, &["data"], true).as_deref(),
            Some("{ data: import('./$types.js').PageData } & Record<string, any>")
        );
        assert_eq!(
            synthesize_route_props_type(RouteKind::Page, &["heading"], true).as_deref(),
            Some("Record<string, any>")
        );
    }

    #[test]
    fn synth_returns_none_when_nothing_is_typed() {
        assert_eq!(
            synthesize_route_props_type(RouteKind::Page, &["heading"], false),
            None
        );
        assert_eq!(
            synthesize_route_props_type(RouteKind::Error, &["data"], false),
            None
        );
        assert_eq!(
            synthesize_route_props_type(RouteKind::Error, &["error"], false).as_deref(),
            Some("{ error: App.Error }")
        );
    }
}
