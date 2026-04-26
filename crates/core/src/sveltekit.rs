//! SvelteKit path classification — single source of truth.
//!
//! Five sites in this repo previously implemented overlapping subsets of
//! "is this a kit file? what kind?" and "how do we normalise a kit
//! path?" rules: `cli/kit_files.rs`, `cli/svelte_config.rs`,
//! `emit/kit_inject.rs`, `emit/sveltekit.rs`, and
//! `typecheck/kit_types_mirror.rs`. Round 3 F4, Round 4 G4 / G5 each
//! fixed one site without touching the others; the next bug followed
//! the same pattern. The plan in `notes/PLAN-sveltekit-path-centralization.md`
//! drives the consolidation.
//!
//! ### What lives here
//!
//! - The recognition rules (basenames, suffixes, hooks dir-form,
//!   params filtering) for every kind of SvelteKit-aware file.
//! - The `KitFilesSettings` struct that carries `kit.files` overrides.
//! - The `normalise_path` helper for `kit.files` path strings.
//! - The `user_source_needles` catalogue used by
//!   `kit_types_mirror`'s chain rewriter.
//!
//! ### What does NOT live here
//!
//! Anything that needs `oxc_ast`, `oxc_parser`, or `walkdir` stays in
//! its consumer crate. This module is `&Path` / `&str` in, owned data
//! out — no AST analysis, no I/O, no allocator.
//!
//! Specifically, the following stay where they are today:
//!
//! - `kit_inject`'s actual `: import('./$types.js').…` splicing logic
//!   (uses `oxc_parser` to find handler-param spans).
//! - `sveltekit.rs`'s `kit_prop_decl` / `kit_widen_type` /
//!   `synthesize_route_props_type` (string formatting that depends on
//!   the classifier output but doesn't classify anything itself).
//! - `svelte_config.rs`'s `defineConfig({...})` / `satisfies` /
//!   `as T` wrapper unwrapping (AST-driven).
//! - `kit_types_mirror`'s rewrite walker (string scanning + filesystem
//!   write).
//!
//! ### `.js` route file note
//!
//! `+page.js` / `+layout.js` / `+server.js` are recognised by
//! [`classify`] (`lang == ScriptLang::Js`). Today, `kit_inject` only
//! injects type annotations into `.ts` files — `.js` overlays would
//! need JSDoc-form annotations, which is a separate code path. The
//! classifier doesn't filter `.js` out at this layer; downstream
//! consumers gate on `lang` if they care.

use std::borrow::Cow;
use std::path::Path;

/// Resolved kit.files settings. Defaults match upstream svelte-check's
/// `defaultKitFilesSettings`. Custom values come from
/// `svelte.config.js`'s `kit.files` block, parsed by
/// `cli::svelte_config`.
///
/// Note: `kit.files.routes` is intentionally absent — upstream's
/// `loadKitFilesSettings` doesn't read it either, and recognition
/// stays basename-only regardless of where the user relocated the
/// routes directory. Diverging here would put us out of parity with
/// upstream's `<N> FILES` denominator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitFilesSettings {
    pub params_path: String,
    pub server_hooks_path: String,
    pub client_hooks_path: String,
    pub universal_hooks_path: String,
}

impl Default for KitFilesSettings {
    fn default() -> Self {
        Self {
            params_path: "src/params".into(),
            server_hooks_path: "src/hooks.server".into(),
            client_hooks_path: "src/hooks.client".into(),
            universal_hooks_path: "src/hooks".into(),
        }
    }
}

/// Script-language flag observed at classification time. Drives
/// downstream "should we run kit_inject here?" gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptLang {
    /// `.ts` — kit_inject injects `: import('./$types.js').…` annotations.
    Ts,
    /// `.js` — kit_inject doesn't run today (JSDoc form is a separate
    /// code path); the file passes through to tsgo as-is.
    Js,
    /// `.svelte` — handled by emit's overlay pipeline, not kit_inject.
    Svelte,
}

/// Page/Layout/Error sub-kind for `.svelte` route component files.
/// Mirrors the `RouteKind` enum that `emit/sveltekit.rs` uses for
/// kit-prop autotyping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RouteShape {
    Page,
    Layout,
    Error,
}

/// Server / client / universal flavour for `.ts` / `.js` route scripts.
/// Mirrors the `{ is_layout, is_server }` pair `emit/kit_inject.rs`
/// uses for `LoadEvent` naming.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptFlavour {
    pub is_layout: bool,
    pub is_server: bool,
}

/// Server / client / universal scope for hooks files.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HooksScope {
    Server,
    Client,
    Universal,
}

/// Top-level role of a SvelteKit file. Drives which downstream pass
/// acts on the file:
///
/// - `RouteComponent` → emit's overlay pipeline (`sveltekit.rs`).
/// - `RouteScript` / `ServerEndpoint` → `kit_inject`.
/// - `Hooks` / `Params` → discovery only today; `kit_inject` doesn't
///   type them yet (parity with upstream's `defaultKitFilesSettings`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KitRole {
    /// `+page.svelte` / `+layout.svelte` / `+error.svelte`.
    RouteComponent { shape: RouteShape },
    /// `+page.ts` / `+layout.ts` / `+page.server.ts` /
    /// `+layout.server.ts` (and `.js` equivalents).
    RouteScript { flavour: ScriptFlavour },
    /// `+server.ts` / `+server.js`.
    ServerEndpoint,
    /// `src/hooks.{server,client,…}.{ts,js}` — including the
    /// directory form (`src/hooks.server/index.ts`).
    Hooks { scope: HooksScope },
    /// `src/params/<matcher>.{ts,js}` (excluding `.test` / `.spec`).
    Params,
}

/// One-shot classification result. Returned by [`classify`]; `None`
/// when the path matches no SvelteKit convention.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KitFile {
    pub role: KitRole,
    pub lang: ScriptLang,
    /// Route-group label parsed from a `+page@(group).svelte` /
    /// `+layout@admin.ts` basename, without the leading `@`. `None`
    /// when no `@…` segment is present, or for non-route roles.
    pub group: Option<String>,
}

/// Classify `path` against `settings`. Returns `Some(KitFile)` for any
/// recognised SvelteKit convention, or `None` for plain user files.
///
/// Consumers that want a boolean "is this a kit file in my bucket?"
/// check filter the result on `role` — for example:
///
/// ```
/// # use svn_core::sveltekit::{classify, KitFilesSettings, KitRole};
/// # use std::path::Path;
/// let s = KitFilesSettings::default();
/// let is_kit_ts_or_js = |p: &Path| {
///     classify(p, &s).is_some_and(|k| !matches!(k.role, KitRole::RouteComponent { .. }))
/// };
/// assert!(is_kit_ts_or_js(Path::new("src/routes/+page.ts")));
/// assert!(!is_kit_ts_or_js(Path::new("src/routes/+page.svelte")));
/// ```
pub fn classify(path: &Path, settings: &KitFilesSettings) -> Option<KitFile> {
    let basename = path.file_name()?.to_str()?;
    let path_str = path.to_str()?;
    let normalised = normalise_path_seps(path_str);

    if let Some(kit) = classify_route(basename) {
        return Some(kit);
    }
    if let Some(kit) = classify_hooks(&normalised, basename, settings) {
        return Some(kit);
    }
    if let Some(kit) = classify_params(&normalised, basename, settings) {
        return Some(kit);
    }
    None
}

/// Route-component (`.svelte`), route-script (`.ts`/`.js`), and server-
/// endpoint (`.ts`/`.js`) classification. Pure basename rule — does
/// not consult `KitFilesSettings` because routes-path is not honoured
/// (see `KitFilesSettings` doc comment).
fn classify_route(basename: &str) -> Option<KitFile> {
    // Split the basename into (stem-before-`@`, `@group` label, ext).
    // `+page@(auth).svelte` → ("+page", Some("(auth)"), "svelte").
    let (rest, ext) = split_extension(basename)?;
    let (stem, group) = split_group(rest);

    let lang = match ext {
        "svelte" => ScriptLang::Svelte,
        "ts" => ScriptLang::Ts,
        "js" => ScriptLang::Js,
        _ => return None,
    };

    let role = match (lang, stem) {
        (ScriptLang::Svelte, "+page") => KitRole::RouteComponent {
            shape: RouteShape::Page,
        },
        (ScriptLang::Svelte, "+layout") => KitRole::RouteComponent {
            shape: RouteShape::Layout,
        },
        (ScriptLang::Svelte, "+error") => KitRole::RouteComponent {
            shape: RouteShape::Error,
        },
        (ScriptLang::Ts | ScriptLang::Js, "+server") => KitRole::ServerEndpoint,
        (ScriptLang::Ts | ScriptLang::Js, "+page") => KitRole::RouteScript {
            flavour: ScriptFlavour {
                is_layout: false,
                is_server: false,
            },
        },
        (ScriptLang::Ts | ScriptLang::Js, "+layout") => KitRole::RouteScript {
            flavour: ScriptFlavour {
                is_layout: true,
                is_server: false,
            },
        },
        (ScriptLang::Ts | ScriptLang::Js, "+page.server") => KitRole::RouteScript {
            flavour: ScriptFlavour {
                is_layout: false,
                is_server: true,
            },
        },
        (ScriptLang::Ts | ScriptLang::Js, "+layout.server") => KitRole::RouteScript {
            flavour: ScriptFlavour {
                is_layout: true,
                is_server: true,
            },
        },
        _ => return None,
    };

    Some(KitFile {
        role,
        lang,
        group: group.map(|s| s.to_string()),
    })
}

/// Hooks classification. Matches both the extension form
/// (`src/hooks.server.ts`) and the directory-index form
/// (`src/hooks.server/index.ts`). The `path` argument is the
/// forward-slash-normalised full path string; `basename` is the
/// stripped filename. Settings provide the per-scope path roots.
fn classify_hooks(path: &str, basename: &str, settings: &KitFilesSettings) -> Option<KitFile> {
    let lang = lang_from_basename(basename)?;
    if !matches!(lang, ScriptLang::Ts | ScriptLang::Js) {
        return None;
    }

    // Probe each scope's configured path. Order doesn't matter
    // semantically (paths are non-overlapping in any sane config) but
    // is preserved for deterministic test output: server, client,
    // universal — same order upstream walks.
    let scopes = [
        (HooksScope::Server, settings.server_hooks_path.as_str()),
        (HooksScope::Client, settings.client_hooks_path.as_str()),
        (
            HooksScope::Universal,
            settings.universal_hooks_path.as_str(),
        ),
    ];

    for (scope, hooks_path) in scopes {
        if path_ends_with_hooks_path(path, basename, hooks_path) {
            return Some(KitFile {
                role: KitRole::Hooks { scope },
                lang,
                group: None,
            });
        }
    }
    None
}

/// Hooks suffix-match: full path stem ends with `hooks_path`, OR
/// `basename == "index.{ts,js}"` and the parent directory ends with
/// `hooks_path`. Kept as a free helper so the test surface is direct
/// — the previous implementation had this inline in `is_hooks_file`.
fn path_ends_with_hooks_path(path: &str, basename: &str, hooks_path: &str) -> bool {
    if (basename == "index.ts" || basename == "index.js")
        && let Some(dir_end) = path.len().checked_sub(basename.len() + 1)
        && path[..dir_end].ends_with(hooks_path)
    {
        return true;
    }
    // Strip the extension (last `.ext`) and check the path stem.
    let Some(ext_idx) = basename.rfind('.') else {
        return false;
    };
    let ext_len = basename.len() - ext_idx;
    let Some(stem_end) = path.len().checked_sub(ext_len) else {
        return false;
    };
    path[..stem_end].ends_with(hooks_path)
}

/// Params classification. Excludes `.test` / `.spec` variants
/// (matches upstream `isParamsFile`). The `path` argument is the
/// forward-slash-normalised full path string.
fn classify_params(path: &str, basename: &str, settings: &KitFilesSettings) -> Option<KitFile> {
    let lang = lang_from_basename(basename)?;
    if !matches!(lang, ScriptLang::Ts | ScriptLang::Js) {
        return None;
    }
    if basename.contains(".test") || basename.contains(".spec") {
        return None;
    }
    let dir_end = path.len().checked_sub(basename.len() + 1)?;
    if !path[..dir_end].ends_with(&settings.params_path) {
        return None;
    }
    Some(KitFile {
        role: KitRole::Params,
        lang,
        group: None,
    })
}

/// Split a basename into `(stem-up-to-last-dot, ext-without-dot)`,
/// or `None` when the basename has no `.`.
fn split_extension(basename: &str) -> Option<(&str, &str)> {
    let dot = basename.rfind('.')?;
    Some((&basename[..dot], &basename[dot + 1..]))
}

/// Split the route stem into `(real-stem, optional-group-label)`.
/// `+page@(auth)` → ("+page", Some("(auth)")). The stem may also
/// have an internal `.` for `+page.server` / `+layout.server`; the
/// `@` is always before that, so a `find('@')` (left-to-right) is
/// the right choice.
fn split_group(stem_with_group: &str) -> (&str, Option<&str>) {
    match stem_with_group.find('@') {
        Some(at) => (&stem_with_group[..at], Some(&stem_with_group[at + 1..])),
        None => (stem_with_group, None),
    }
}

/// Determine `ScriptLang` purely from the basename's extension.
/// Returns `None` when the extension isn't one of the recognised
/// script extensions.
fn lang_from_basename(basename: &str) -> Option<ScriptLang> {
    if basename.ends_with(".ts") {
        Some(ScriptLang::Ts)
    } else if basename.ends_with(".js") {
        Some(ScriptLang::Js)
    } else if basename.ends_with(".svelte") {
        Some(ScriptLang::Svelte)
    } else {
        None
    }
}

/// Normalise a `kit.files` path string into the canonical shape
/// `classify`'s suffix matchers expect: drop a leading `./` and a
/// trailing `/`. Without this, a user-written `./src/myparams` would
/// never match an absolute walker path because the `./` prefix has
/// no analogue in the normalised path.
///
/// Moves from `cli::svelte_config::normalise_kit_path` so the
/// settings-population code in `cli/` and the suffix-match code here
/// share a single normalisation rule.
pub fn normalise_path(s: &str) -> String {
    s.trim_start_matches("./").trim_end_matches('/').to_string()
}

/// Path-separator normalisation used internally. Backslash-bearing
/// Windows paths (`C:\repo\src\hooks.server.ts`) need conversion
/// before suffix-matching against forward-slash settings strings
/// (`src/hooks.server`). This isn't part of the public API — exposing
/// it would invite consumers to perform double-normalisation; instead
/// callers feed `&Path` to `classify` and the function handles it.
fn normalise_path_seps(path: &str) -> Cow<'_, str> {
    if path.contains('\\') {
        Cow::Owned(path.replace('\\', "/"))
    } else {
        Cow::Borrowed(path)
    }
}

/// Catalogue of "user-source segment" needles for
/// `kit_types_mirror`'s chain rewriter. Used to detect import-chain
/// substrings inside `$types.d.ts` files that point at the user's
/// real source tree (which we then redirect to the cache mirror).
///
/// Today only the `src/routes/` segment is rewritten — see
/// `kit_types_mirror::rewrite_user_source_chain`'s doc comment for
/// why hooks/params chains are left alone. The list is generated
/// from settings (instead of hardcoded) so when `kit_inject` learns
/// to type hooks/params, expanding the mirror is a one-line change
/// here rather than a parallel hardcoded list elsewhere.
///
/// The trailing `/` on each entry is load-bearing: the mirror's
/// rewriter needs to insert `svelte/` BEFORE the segment, and the
/// `/` boundary distinguishes `src/routes/` from a
/// `src/routes-extra/` typo.
pub fn user_source_needles(_settings: &KitFilesSettings) -> Vec<String> {
    // Settings-derived hooks/params entries are intentionally NOT
    // emitted — kit_inject doesn't materialise cache copies for
    // those, so the rewriter must leave them pointing at user-tree.
    // The `_settings` parameter is kept on the signature so callers
    // pass it through (and so a future expansion that needs the
    // configured params/hooks paths doesn't require a signature
    // bump on every consumer).
    vec!["src/routes/".to_string()]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    fn settings() -> KitFilesSettings {
        KitFilesSettings::default()
    }

    // ----- ScriptLang / extension splits ------------------------

    #[test]
    fn lang_from_basename_recognises_canonical_extensions() {
        assert_eq!(lang_from_basename("foo.ts"), Some(ScriptLang::Ts));
        assert_eq!(lang_from_basename("foo.js"), Some(ScriptLang::Js));
        assert_eq!(lang_from_basename("foo.svelte"), Some(ScriptLang::Svelte));
        assert_eq!(lang_from_basename("foo.json"), None);
        assert_eq!(lang_from_basename("foo"), None);
    }

    #[test]
    fn split_group_extracts_label_after_at() {
        assert_eq!(split_group("+page@(auth)"), ("+page", Some("(auth)")));
        assert_eq!(split_group("+layout@admin"), ("+layout", Some("admin")));
        assert_eq!(split_group("+page"), ("+page", None));
    }

    // ----- Route components (.svelte) ---------------------------

    #[test]
    fn page_svelte_classifies_as_route_component_page() {
        let kit = classify(&p("src/routes/+page.svelte"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteComponent {
                shape: RouteShape::Page
            }
        );
        assert_eq!(kit.lang, ScriptLang::Svelte);
        assert!(kit.group.is_none());
    }

    #[test]
    fn layout_svelte_classifies_as_route_component_layout() {
        let kit = classify(&p("src/routes/+layout.svelte"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteComponent {
                shape: RouteShape::Layout
            }
        );
    }

    #[test]
    fn error_svelte_classifies_as_route_component_error() {
        let kit = classify(&p("src/routes/+error.svelte"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteComponent {
                shape: RouteShape::Error
            }
        );
    }

    #[test]
    fn route_component_strips_group_suffix() {
        let kit = classify(&p("src/routes/(auth)/+page@(auth).svelte"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteComponent {
                shape: RouteShape::Page
            }
        );
        assert_eq!(kit.group.as_deref(), Some("(auth)"));
    }

    #[test]
    fn non_route_svelte_returns_none() {
        assert!(classify(&p("src/routes/Component.svelte"), &settings()).is_none());
        assert!(classify(&p("src/routes/page.svelte"), &settings()).is_none()); // no leading `+`
    }

    // ----- Route scripts (.ts / .js) ----------------------------

    #[test]
    fn page_ts_classifies_as_route_script_page() {
        let kit = classify(&p("src/routes/+page.ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteScript {
                flavour: ScriptFlavour {
                    is_layout: false,
                    is_server: false,
                }
            }
        );
        assert_eq!(kit.lang, ScriptLang::Ts);
    }

    #[test]
    fn layout_server_ts_classifies_with_both_flavour_flags() {
        let kit = classify(&p("src/routes/+layout.server.ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteScript {
                flavour: ScriptFlavour {
                    is_layout: true,
                    is_server: true,
                }
            }
        );
    }

    #[test]
    fn page_js_classifies_as_route_script_with_lang_js() {
        let kit = classify(&p("src/routes/+page.js"), &settings()).unwrap();
        assert!(matches!(kit.role, KitRole::RouteScript { .. }));
        assert_eq!(kit.lang, ScriptLang::Js);
    }

    #[test]
    fn server_ts_classifies_as_server_endpoint() {
        let kit = classify(&p("src/routes/api/+server.ts"), &settings()).unwrap();
        assert_eq!(kit.role, KitRole::ServerEndpoint);
        assert_eq!(kit.lang, ScriptLang::Ts);
    }

    #[test]
    fn route_script_strips_group_suffix() {
        let kit = classify(&p("src/routes/+page@(auth).ts"), &settings()).unwrap();
        assert!(matches!(kit.role, KitRole::RouteScript { .. }));
        assert_eq!(kit.group.as_deref(), Some("(auth)"));
    }

    #[test]
    fn page_server_with_group_classifies_correctly() {
        // `+page.server@(authed).ts` — `@group` precedes the inner `.server`.
        let kit = classify(&p("src/routes/+page.server@(authed).ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::RouteScript {
                flavour: ScriptFlavour {
                    is_layout: false,
                    is_server: true,
                }
            }
        );
        assert_eq!(kit.group.as_deref(), Some("(authed)"));
    }

    // ----- Hooks files ------------------------------------------

    #[test]
    fn hooks_server_ts_classifies_via_extension_form() {
        let kit = classify(&p("src/hooks.server.ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::Hooks {
                scope: HooksScope::Server
            }
        );
    }

    #[test]
    fn hooks_client_ts_classifies() {
        let kit = classify(&p("src/hooks.client.ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::Hooks {
                scope: HooksScope::Client
            }
        );
    }

    #[test]
    fn hooks_universal_ts_classifies() {
        let kit = classify(&p("src/hooks.ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::Hooks {
                scope: HooksScope::Universal
            }
        );
    }

    #[test]
    fn hooks_dir_index_form_classifies() {
        let kit = classify(&p("src/hooks.server/index.ts"), &settings()).unwrap();
        assert_eq!(
            kit.role,
            KitRole::Hooks {
                scope: HooksScope::Server
            }
        );
    }

    #[test]
    fn hooks_universal_does_not_match_more_specific_paths() {
        // `src/hooks.server.ts` must classify as Server, not Universal,
        // even though the universal hooks path is a prefix of the
        // server one. Probe order ensures Server wins.
        let kit = classify(&p("src/hooks.server.ts"), &settings()).unwrap();
        assert!(!matches!(
            kit.role,
            KitRole::Hooks {
                scope: HooksScope::Universal
            }
        ));
    }

    #[test]
    fn hooks_in_other_directory_does_not_classify() {
        assert!(classify(&p("src/other/hooks.ts"), &settings()).is_none());
    }

    #[test]
    fn hooks_with_extra_text_does_not_classify() {
        assert!(classify(&p("src/hooks-extra.ts"), &settings()).is_none());
    }

    #[test]
    fn hooks_custom_path_override() {
        let s = KitFilesSettings {
            server_hooks_path: "src/server/hooks.server".into(),
            ..KitFilesSettings::default()
        };
        let kit = classify(&p("src/server/hooks.server.ts"), &s).unwrap();
        assert_eq!(
            kit.role,
            KitRole::Hooks {
                scope: HooksScope::Server
            }
        );
        // Default-location hook NO LONGER classifies under the override
        // — but the universal default ("src/hooks") still applies, and
        // `src/hooks.server.ts` doesn't end with `src/hooks` (the stem
        // ends with `.server`), so it falls through.
        assert!(classify(&p("src/hooks.server.ts"), &s).is_none());
    }

    // ----- Params files -----------------------------------------

    #[test]
    fn params_ts_classifies() {
        let kit = classify(&p("src/params/videoId.ts"), &settings()).unwrap();
        assert_eq!(kit.role, KitRole::Params);
    }

    #[test]
    fn params_js_classifies() {
        let kit = classify(&p("src/params/channelId.js"), &settings()).unwrap();
        assert_eq!(kit.role, KitRole::Params);
        assert_eq!(kit.lang, ScriptLang::Js);
    }

    #[test]
    fn params_excludes_test_and_spec_variants() {
        assert!(classify(&p("src/params/videoId.test.ts"), &settings()).is_none());
        assert!(classify(&p("src/params/videoId.spec.ts"), &settings()).is_none());
    }

    #[test]
    fn params_custom_path_override() {
        let s = KitFilesSettings {
            params_path: "src/myparams".into(),
            ..KitFilesSettings::default()
        };
        assert_eq!(
            classify(&p("src/myparams/foo.ts"), &s).unwrap().role,
            KitRole::Params
        );
        // Default location no longer matches under override.
        assert!(classify(&p("src/params/foo.ts"), &s).is_none());
    }

    // ----- Windows path normalisation ---------------------------

    #[test]
    fn windows_paths_via_internal_helpers() {
        // `PathBuf` on Unix doesn't split on backslashes (so we can't
        // build a real PathBuf from `r"C:\…"`), but the internal
        // suffix-matchers exercise the normalised form directly.
        let n = normalise_path_seps(r"C:\repo\src\hooks.server.ts");
        assert!(path_ends_with_hooks_path(
            &n,
            "hooks.server.ts",
            &settings().server_hooks_path
        ));
    }

    #[test]
    fn normalise_path_seps_borrows_when_clean() {
        match normalise_path_seps("/abs/src/foo.ts") {
            Cow::Borrowed(_) => {}
            Cow::Owned(_) => panic!("clean path should borrow"),
        }
    }

    // ----- normalise_path (kit.files string form) ---------------

    #[test]
    fn normalise_path_strips_dotslash_prefix() {
        assert_eq!(normalise_path("./src/myparams"), "src/myparams");
    }

    #[test]
    fn normalise_path_strips_trailing_slash() {
        assert_eq!(normalise_path("src/myparams/"), "src/myparams");
    }

    #[test]
    fn normalise_path_strips_both() {
        assert_eq!(normalise_path("./src/myparams/"), "src/myparams");
    }

    #[test]
    fn normalise_path_leaves_clean_input_alone() {
        assert_eq!(normalise_path("src/myparams"), "src/myparams");
    }

    // ----- user_source_needles ----------------------------------

    #[test]
    fn user_source_needles_default_is_routes_only() {
        let needles = user_source_needles(&settings());
        assert_eq!(needles, vec!["src/routes/".to_string()]);
    }

    #[test]
    fn user_source_needles_does_not_include_hooks_or_params() {
        // Hooks and params are intentionally absent — see the doc
        // comment on `user_source_needles` for the rationale.
        let needles = user_source_needles(&settings());
        for n in &needles {
            assert!(!n.contains("hooks"));
            assert!(!n.contains("params"));
        }
    }

    // ----- Cross-cutting non-kit cases --------------------------

    #[test]
    fn plain_user_files_return_none() {
        assert!(classify(&p("src/lib/foo.ts"), &settings()).is_none());
        assert!(classify(&p("src/lib/Component.svelte"), &settings()).is_none());
        assert!(classify(&p("src/routes/helper.ts"), &settings()).is_none());
    }
}
