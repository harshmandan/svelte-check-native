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

/// The kind of SvelteKit route file a basename matches.
///
/// Pattern matching mirrors upstream's `isKitRouteFile`:
/// `+page`, `+layout`, `+page.server`, `+layout.server`, `+server`, `+error`.
/// `@<layout-name>` suffixes are stripped before matching so
/// `+page@foo.svelte` still resolves to `Page`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteKind {
    /// `+page.svelte` — page component. `data: PageData`, `form: ActionData`.
    Page,
    /// `+layout.svelte` — layout component. `data: LayoutData`,
    /// `children: Snippet` implicitly present.
    Layout,
    /// `+error.svelte` — error boundary. No auto-typed props today.
    Error,
}

/// Inspect the basename and return a `RouteKind` when it matches a
/// SvelteKit route pattern, or `None` otherwise.
///
/// We intentionally recognize only `.svelte` route files here:
/// `+page.ts`, `+server.ts`, and their friends reach tsgo as raw
/// user-authored TypeScript and aren't transformed through emit.
pub fn route_kind(path: &Path) -> Option<RouteKind> {
    let basename = path.file_name()?.to_str()?;
    if !basename.ends_with(".svelte") {
        return None;
    }
    // Strip `.svelte` and any `@layout-ref` suffix on the stem
    // (`+page@(foo).svelte` → `+page`).
    let stem = basename.strip_suffix(".svelte")?;
    let core = match stem.find('@') {
        Some(at) => &stem[..at],
        None => stem,
    };
    match core {
        "+page" => Some(RouteKind::Page),
        "+layout" => Some(RouteKind::Layout),
        "+error" => Some(RouteKind::Error),
        _ => None,
    }
}

/// Return the full property declaration (including the optional `?`
/// marker and the type source) for a kit-auto-typed prop name, or
/// `None` when `name` is not auto-typed for this kind. Rendered as
/// `<name>[?]: <type>`.
///
/// Conservative by design: props this function doesn't recognize fall
/// back to the existing `<name>?: any` shape in the caller. That keeps
/// user-defined props like `let { data, heading } = $props()` in a
/// route file working — `heading` stays `any`, `data` becomes
/// `PageData`.
///
/// `params` is intentionally NOT auto-typed. Upstream emits
/// `import('./$types.js').PageProps['params']`, but `PageProps` was
/// only standardized in SvelteKit 2.16+; older projects that predate
/// it fire TS2694 ("has no exported member 'PageProps'"). The
/// user-defined-type fallback (`any`) is safe.
pub fn kit_prop_decl(name: &str, kind: RouteKind) -> Option<String> {
    match (kind, name) {
        (RouteKind::Page, "data") => Some("data: import('./$types.js').PageData".into()),
        (RouteKind::Page, "form") => Some("form?: import('./$types.js').ActionData".into()),
        (RouteKind::Layout, "data") => Some("data: import('./$types.js').LayoutData".into()),
        (RouteKind::Layout, "children") => Some("children?: import('svelte').Snippet".into()),
        // `+error.svelte` doesn't receive props via $props(); its shape
        // comes from `page.error`. Leave it unannotated for now.
        _ => None,
    }
}

/// Return just the TYPE source (no name, no `:`) for a Kit-auto-typed
/// Svelte-4 `export let <name>` declaration on a route file. The
/// caller splices `: <type>` after the identifier in the overlay.
///
/// Mirrors upstream `svelte2tsx/src/svelte2tsx/nodes/ExportedNames.ts`
/// `handleTypeAssertion` (lines 424-440): when the exported local is
/// one of `data` / `form` / `snapshot` on a Kit route file AND the
/// user didn't already annotate it, upstream synthesizes
/// `: import('./$types.js').<Type>`. We match the same set but widen
/// `form`/`snapshot` with `| undefined` because `let X: T;` can't
/// carry TS's object-member `?` optional marker — the declaration
/// needs a value-position `T | undefined` union. `data` stays
/// required (upstream emits `: PageData` without `| undefined` since
/// the reassignment via `__sveltets_2_any(data)` loosens it
/// downstream anyway; our `!` definite-assign has the same net
/// effect).
///
/// Returns `None` for names that aren't kit-auto-typed — the caller
/// falls back to `: any` (our legacy widen).
pub fn kit_widen_type(name: &str, kind: RouteKind) -> Option<&'static str> {
    match (kind, name) {
        (RouteKind::Page, "data") => Some("import('./$types.js').PageData"),
        (RouteKind::Page, "form") => Some("import('./$types.js').ActionData | undefined"),
        (RouteKind::Page, "snapshot") => Some("import('./$types.js').Snapshot | undefined"),
        (RouteKind::Layout, "data") => Some("import('./$types.js').LayoutData"),
        (RouteKind::Layout, "snapshot") => Some("import('./$types.js').Snapshot | undefined"),
        _ => None,
    }
}

/// Build the synthesized Props object type for a route-file `.svelte`
/// that has no explicit `$props()` annotation. Returns `None` when no
/// prop in the destructure list is kit-auto-typed; the caller then
/// continues with the existing "no annotation → default = any" path.
///
/// Unrecognized props in the destructure (user-defined) are emitted as
/// `<name>?: any;` so the synthesized shape stays a superset of what
/// the user wrote; marking them optional avoids TS2741 errors at the
/// component-instantiation sites where the user's template doesn't
/// pass them. Matches the convention `PropsInfo::build` uses for
/// untyped `export let foo = default` declarations (see
/// `svn_analyze::props`).
///
/// For `+layout.svelte`, `children` is added implicitly even when the
/// user doesn't destructure it (upstream does this too). Layouts
/// always receive a `children` snippet from SvelteKit at runtime.
pub fn synthesize_route_props_type(kind: RouteKind, prop_names: &[&str]) -> Option<String> {
    use std::fmt::Write;

    // Two-pass so we can bail with `None` without allocating the
    // output buffer if nothing Kit-specific landed. First pass
    // classifies each prop + tracks whether a Layout's implicit
    // `children` slot is already covered.
    let mut saw_kit_prop = false;
    let mut saw_children = false;
    for &name in prop_names {
        if kit_prop_decl(name, kind).is_some() {
            saw_kit_prop = true;
            if name == "children" {
                saw_children = true;
            }
        }
    }
    let need_implicit_children = matches!(kind, RouteKind::Layout) && !saw_children;
    if !saw_kit_prop && !need_implicit_children {
        return None;
    }

    // Second pass: write directly into the output buffer via `write!`,
    // no intermediate `Vec<String>`. Capacity is a rough overestimate
    // (avg ~28 bytes per declaration + separators) — beats a
    // per-item `format!` allocation.
    let mut out = String::with_capacity(prop_names.len() * 40 + 32);
    out.push_str("{ ");
    let mut first = true;
    let push_sep = |buf: &mut String, first: &mut bool| {
        if !*first {
            buf.push(' ');
        }
        *first = false;
    };
    for &name in prop_names {
        if let Some(decl) = kit_prop_decl(name, kind) {
            push_sep(&mut out, &mut first);
            let _ = write!(out, "{decl};");
        } else {
            // Preserve user-defined props with the conservative `?: any`
            // shape. Losing them from the synthesized type would shrink
            // the overlay default's Props and fire spurious
            // "Property 'foo' does not exist" at callers that pass
            // `foo` down from a parent layout's data flow.
            push_sep(&mut out, &mut first);
            let _ = write!(out, "{name}?: any;");
        }
    }
    if need_implicit_children {
        push_sep(&mut out, &mut first);
        out.push_str("children?: import('svelte').Snippet;");
    }
    out.push_str(" }");
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn kind_of(basename: &str) -> Option<RouteKind> {
        route_kind(&PathBuf::from(basename))
    }

    #[test]
    fn detects_page_svelte() {
        assert_eq!(kind_of("+page.svelte"), Some(RouteKind::Page));
    }

    #[test]
    fn detects_layout_svelte() {
        assert_eq!(kind_of("+layout.svelte"), Some(RouteKind::Layout));
    }

    #[test]
    fn detects_error_svelte() {
        assert_eq!(kind_of("+error.svelte"), Some(RouteKind::Error));
    }

    #[test]
    fn strips_at_suffix_on_stem() {
        assert_eq!(kind_of("+page@(auth).svelte"), Some(RouteKind::Page));
        assert_eq!(kind_of("+layout@foo.svelte"), Some(RouteKind::Layout));
    }

    #[test]
    fn rejects_ts_route_files() {
        // .ts files go to tsgo directly — they're not our overlay concern.
        assert_eq!(kind_of("+page.ts"), None);
        assert_eq!(kind_of("+server.ts"), None);
        assert_eq!(kind_of("+page.server.ts"), None);
    }

    #[test]
    fn rejects_non_route_svelte() {
        assert_eq!(kind_of("Component.svelte"), None);
        assert_eq!(kind_of("page.svelte"), None); // no leading '+'
    }

    #[test]
    fn kit_prop_decl_page_data_required() {
        assert_eq!(
            kit_prop_decl("data", RouteKind::Page).as_deref(),
            Some("data: import('./$types.js').PageData")
        );
    }

    #[test]
    fn kit_prop_decl_page_form_optional() {
        assert_eq!(
            kit_prop_decl("form", RouteKind::Page).as_deref(),
            Some("form?: import('./$types.js').ActionData")
        );
    }

    #[test]
    fn kit_prop_decl_layout_data() {
        assert_eq!(
            kit_prop_decl("data", RouteKind::Layout).as_deref(),
            Some("data: import('./$types.js').LayoutData")
        );
    }

    #[test]
    fn kit_prop_decl_layout_children() {
        assert_eq!(
            kit_prop_decl("children", RouteKind::Layout).as_deref(),
            Some("children?: import('svelte').Snippet")
        );
    }

    #[test]
    fn kit_prop_decl_params_left_as_any() {
        // We don't auto-type params — upstream uses PageProps['params']
        // which requires SvelteKit 2.16+. Safer to skip.
        assert_eq!(kit_prop_decl("params", RouteKind::Page), None);
    }

    #[test]
    fn synth_page_with_data_only() {
        let ty = synthesize_route_props_type(RouteKind::Page, &["data"]).unwrap();
        assert_eq!(ty, "{ data: import('./$types.js').PageData; }");
    }

    #[test]
    fn synth_page_with_data_form_user_prop() {
        let ty =
            synthesize_route_props_type(RouteKind::Page, &["data", "form", "heading"]).unwrap();
        assert_eq!(
            ty,
            "{ data: import('./$types.js').PageData; form?: import('./$types.js').ActionData; \
             heading?: any; }"
        );
    }

    #[test]
    fn synth_layout_injects_children_when_missing() {
        let ty = synthesize_route_props_type(RouteKind::Layout, &["data"]).unwrap();
        assert_eq!(
            ty,
            "{ data: import('./$types.js').LayoutData; \
             children?: import('svelte').Snippet; }"
        );
    }

    #[test]
    fn synth_layout_preserves_explicit_children() {
        let ty = synthesize_route_props_type(RouteKind::Layout, &["data", "children"]).unwrap();
        assert_eq!(
            ty,
            "{ data: import('./$types.js').LayoutData; \
             children?: import('svelte').Snippet; }"
        );
    }

    #[test]
    fn synth_returns_none_when_no_kit_props() {
        // `+page.svelte` with `let { heading } = $props()` — nothing
        // kit-specific to synthesize; caller falls back to `any`.
        assert_eq!(
            synthesize_route_props_type(RouteKind::Page, &["heading"]),
            None
        );
    }

    #[test]
    fn synth_error_route_returns_none() {
        // `+error.svelte` has no auto-typed props in our mapping.
        assert_eq!(
            synthesize_route_props_type(RouteKind::Error, &["data"]),
            None
        );
    }
}
