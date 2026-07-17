//! SvelteKit Kit-file type injection.
//!
//! Mirrors a subset of upstream svelte2tsx's `upsertKitFile` behavior:
//! for a Kit file whose user source omits a handler's parameter type
//! or a config variable's annotation, splice in the expected
//! `: import('./$types.js').Xxx` / `: boolean | ...` annotation. The
//! result is the original source with insertions at specific byte
//! positions — positions that line up with where the user would have
//! hand-written the annotation, so diagnostic positions map back
//! cleanly.
//!
//! Shipped branches:
//!
//! - `+server.ts` HTTP handlers (`GET` / `POST` / `PUT` / `PATCH` /
//!   `DELETE` / `OPTIONS` / `HEAD` / `fallback`) — inject
//!   `: import('./$types.js').RequestEvent` on the single untyped
//!   parameter, plus a return-type constraint (`Promise<Response>` for
//!   `async`, else `Response | Promise<Response>`) when the handler has
//!   no explicit return type, so returning a non-`Response` value fires.
//! - `+page.ts` / `+layout.ts` / `+page.server.ts` /
//!   `+layout.server.ts`:
//!     - `load` function's first parameter gets
//!       `: import('./$types.js').(Page|Layout)(Server)?LoadEvent` — the
//!       name matrix matches upstream's naming exactly.
//!     - SvelteKit page-option exports (`ssr`, `csr`, `prerender`,
//!       `trailingSlash`) get their fixed value-union types injected
//!       on the declarator binding.
//!
//! `.js` route files receive the same injections in JSDoc form
//! (mirrors upstream `upsertKitFile`'s `isTsFile` split): `@param`
//! blocks for `load` handlers, `@type` casts for page options, a
//! whole-function `@type` for `+server.js` handlers, and `@satisfies`
//! casts for non-function `load` values.
//!
//! Deliberately NOT handled here (yet):
//!
//! - `actions` const satisfies pattern.
//! - `entries` function in `+page.server.ts` / `+server.ts`.
//! - `hooks.server.ts` / `hooks.client.ts` handler typing.
//! - `src/params/*.ts` param matchers.
//! - `+server.ts` page-option / `load` / `actions` / `entries`
//!   typing. The `ServerEndpoint` branch below annotates HTTP
//!   handler parameters and return types; it intentionally skips
//!   page-option consts (`ssr` / `csr` / `prerender` / `trailingSlash`),
//!   `load`, `actions`, and `entries`. Upstream does inject those on
//!   `+server.ts`, so this is a deliberate laxer-than-upstream
//!   divergence for those degenerate cases.

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Declaration, Statement};
use oxc_span::GetSpan;
use std::path::Path;
use svn_core::sveltekit::{KitFilesSettings, KitRole, ScriptLang, classify};
use svn_parser::{ScriptLang as ParserScriptLang, parse_script_body};

/// HTTP method names that `+server.ts` may export as handler functions,
/// per the SvelteKit runtime. Order matches upstream svelte2tsx's
/// `insertApiMethod` sequence for parity.
const SERVER_HANDLER_NAMES: &[&str] = &[
    "GET", "PUT", "POST", "PATCH", "DELETE", "OPTIONS", "HEAD", "fallback",
];

/// Local view onto the centralised classifier — the only shapes
/// `kit_inject` acts on. Built from `svn_core::sveltekit::classify`'s
/// richer `KitRole` so the conversion is one place, not threaded
/// through every match arm.
enum KitFileKind {
    /// `+server.ts` — HTTP handlers get `RequestEvent`. No config
    /// exports (`ssr`/`csr`/etc. are page-only).
    ServerEndpoint,
    /// `+page.ts`, `+layout.ts`, `+page.server.ts`, `+layout.server.ts`.
    /// `load` gets a type-matrix-derived `LoadEvent`; page-option
    /// consts get their fixed-union types. Sub-classification feeds
    /// the load-event name computation.
    Route { is_layout: bool, is_server: bool },
}

/// Classify `path` for kit_inject's purposes. Returns `None` for any
/// shape we don't currently inject into:
///
/// - Hooks / params (recognised by discovery but no annotations
///   injected today).
/// - Plain user files.
///
/// The second tuple element is `true` for `.ts` sources; `.js` route
/// files get the same injections in JSDoc form (mirrors upstream
/// `upsertKitFile`'s `isTsFile` split).
///
/// Defaults are used for `KitFilesSettings` because kit_inject
/// doesn't currently consult per-project overrides — only basename
/// shape matters here, and the route-classification path inside
/// `classify` doesn't read any of the settings fields. Centralising
/// the defaults keeps the call site honest about that fact.
fn kit_file_kind(path: &Path) -> Option<(KitFileKind, bool)> {
    let kit = classify(path, &KitFilesSettings::default())?;
    let is_ts = matches!(kit.lang, ScriptLang::Ts);
    match kit.role {
        KitRole::ServerEndpoint => Some((KitFileKind::ServerEndpoint, is_ts)),
        KitRole::RouteScript { flavour } => Some((
            KitFileKind::Route {
                is_layout: flavour.is_layout,
                is_server: flavour.is_server,
            },
            is_ts,
        )),
        // RouteComponent / Hooks / Params don't get annotations from
        // this pass — return None so the caller skips them.
        _ => None,
    }
}

/// JS-form gate mirroring upstream `findExports` / `hasTypedParameter`
/// for non-TS files: an export whose statement is directly preceded by
/// a JSDoc block carrying `@type` / `@param` / `@satisfies` counts as
/// user-typed, and the injector must leave it alone (upstream checks
/// `ts.getJSDocType` / `getJSDocParameterTags` / a `satisfies` tag).
/// Comments aren't AST, so this is a bounded textual check on the
/// bytes immediately before the statement.
fn has_preceding_jsdoc_typing(source: &str, stmt_start: usize) -> bool {
    let before = source[..stmt_start.min(source.len())].trim_end();
    if !before.ends_with("*/") {
        return false;
    }
    let Some(open) = before.rfind("/**") else {
        return false;
    };
    let block = &before[open..];
    block.contains("@type") || block.contains("@param") || block.contains("@satisfies")
}

/// Returns the modified source with injected type annotations, or
/// `None` if no injections were needed (no handlers matched OR all
/// handlers already carry explicit types).
///
/// The returned string preserves the original source's byte layout
/// except at the insertion points — every insertion is purely
/// additive, so diagnostic positions at lines unaffected by the
/// inject still map 1:1 to the source.
pub fn inject(path: &Path, source: &str) -> Option<String> {
    let (kind, is_ts) = kit_file_kind(path)?;

    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, source, ParserScriptLang::Ts);

    let mut insertions: Vec<(usize, String)> = Vec::new();
    for stmt in &parsed.program.body {
        let Statement::ExportNamedDeclaration(export) = stmt else {
            continue;
        };
        // JS sources: an export the user already JSDoc-typed is
        // upstream's `hasTypeDefinition` — leave it untouched.
        let js_user_typed =
            !is_ts && has_preceding_jsdoc_typing(source, export.span.start as usize);

        match &export.declaration {
            Some(Declaration::FunctionDeclaration(func)) => {
                let Some(name) = func.id.as_ref().map(|id| id.name.as_str()) else {
                    continue;
                };
                if js_user_typed {
                    continue;
                }
                match &kind {
                    KitFileKind::ServerEndpoint => {
                        if !SERVER_HANDLER_NAMES.contains(&name) {
                            continue;
                        }
                        if is_ts {
                            collect_handler_insert(
                                func,
                                "import('./$types.js').RequestEvent",
                                true,
                                &mut insertions,
                            );
                        } else {
                            // JS: one `@type` covering param + return.
                            // Upstream's addTypeToFunction JSDoc branch
                            // builds `(arg0: <type>) => <returnType>`
                            // (the async variant only exists on the TS
                            // path).
                            collect_js_fn_type_insert(
                                func,
                                export.span.start as usize,
                                "(arg0: import('./$types.js').RequestEvent) => Response | Promise<Response>",
                                &mut insertions,
                            );
                        }
                    }
                    KitFileKind::Route {
                        is_layout,
                        is_server,
                    } => {
                        if name != "load" {
                            continue;
                        }
                        let event_type = load_event_type(*is_layout, *is_server);
                        if is_ts {
                            collect_handler_insert(func, &event_type, false, &mut insertions);
                        } else {
                            // JS: `@param` on the (lone) event arg —
                            // upstream's addJsDocParamToFunction. The
                            // return stays untyped, same as the TS
                            // branch.
                            collect_js_param_insert(
                                &func.params,
                                export.span.start as usize,
                                &event_type,
                                &mut insertions,
                            );
                        }
                    }
                }
            }
            Some(Declaration::VariableDeclaration(var_decl)) => {
                let KitFileKind::Route {
                    is_layout,
                    is_server,
                } = &kind
                else {
                    continue;
                };
                // Upstream's findExports only registers single-declarator
                // export const statements (declarations.length === 1); a
                // multi-declarator list is ignored entirely, so skip it
                // here to avoid injecting types upstream never would.
                if var_decl.declarations.len() != 1 {
                    continue;
                }
                for declarator in &var_decl.declarations {
                    if declarator.init.is_none() {
                        continue;
                    }
                    let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                        continue;
                    };
                    if js_user_typed {
                        continue;
                    }

                    // Page-option export (`prerender`, `ssr`, etc.):
                    // splice `: type` after the identifier (TS) or a
                    // JSDoc cast around the initializer (JS —
                    // upstream's addJsDocTypeToVariable).
                    if let Some(annot) = page_option_type(id.name.as_str()) {
                        if declarator.type_annotation.is_some() {
                            continue;
                        }
                        if is_ts {
                            let insert_at = id.span.end as usize;
                            insertions.push((insert_at, format!(": {annot}")));
                        } else if let Some(init) = declarator.init.as_ref() {
                            let s = init.span();
                            insertions
                                .push((s.start as usize, format!("/** @type {{{annot}}} */ (")));
                            insertions.push((s.end as usize, ")".to_string()));
                        }
                        continue;
                    }

                    // Arrow-form `load` (`export const load = async (event) => …`):
                    // mirror the function-form path — find the lone
                    // arrow parameter and splice the load-event
                    // annotation onto it. Without this, users writing
                    // arrow-form `load` lose the SvelteKit-injected
                    // event type and `({ url })` becomes implicit
                    // `any`, firing TS7031 on every parameter
                    // destructure. Upstream's
                    // language-tools/packages/svelte2tsx applies the
                    // same param annotation regardless of declaration
                    // form (function vs const arrow) — see
                    // `getKitTypePath` callers in `incremental.ts`.
                    //
                    // Skip when the user has annotated the variable
                    // (`export const load: Load = ...`). Splicing the
                    // narrower Kit-route event type onto an arrow
                    // already constrained to the broader `Load`
                    // signature creates a contravariant-param mismatch
                    // (TS2322 `({url}: LayoutLoadEvent) => ...` is not
                    // assignable to `Load`). Honour the user's
                    // explicit type — they've taken responsibility for
                    // the param shape themselves.
                    if id.name.as_str() == "load"
                        && declarator.type_annotation.is_none()
                        && let Some(init) = declarator.init.as_ref()
                    {
                        use oxc_ast::ast::Expression;
                        match init {
                            // Arrow-form `load` (`export const load =
                            // async ({…}) => {…}`). The parser exposes
                            // `async` as a flag on the arrow, so no
                            // unwrap is needed for the common case.
                            Expression::ArrowFunctionExpression(arrow) => {
                                let event_type = load_event_type(*is_layout, *is_server);
                                if is_ts {
                                    collect_arrow_handler_insert(
                                        arrow,
                                        &event_type,
                                        &mut insertions,
                                    );
                                } else {
                                    collect_js_param_insert(
                                        &arrow.params,
                                        arrow.span.start as usize,
                                        &event_type,
                                        &mut insertions,
                                    );
                                }
                            }
                            // Function-expression-form `load`
                            // (`export const load = function ({…}) {…}`).
                            Expression::FunctionExpression(func) => {
                                let event_type = load_event_type(*is_layout, *is_server);
                                if is_ts {
                                    collect_handler_insert(
                                        func,
                                        &event_type,
                                        false,
                                        &mut insertions,
                                    );
                                } else {
                                    collect_js_param_insert(
                                        &func.params,
                                        func.span.start as usize,
                                        &event_type,
                                        &mut insertions,
                                    );
                                }
                            }
                            // Already `satisfies`-wrapped — upstream
                            // treats this as user-supplied typing, so
                            // leave it alone to avoid double-wrapping.
                            Expression::TSSatisfiesExpression(_) => {}
                            // Non-function `load` value (e.g. a
                            // re-exported imported loader). Mirror
                            // upstream's `type:'var'` branch: wrap the
                            // value in `(...) satisfies <...>Load` (TS)
                            // or a `@satisfies` JSDoc cast (JS —
                            // upstream's addJsDocSatisfiesToVariable).
                            _ => {
                                let load_ty = load_satisfies_type(*is_layout, *is_server);
                                let s = init.span();
                                if is_ts {
                                    insertions.push((s.start as usize, "(".to_string()));
                                    insertions
                                        .push((s.end as usize, format!(") satisfies {load_ty}")));
                                } else {
                                    insertions.push((
                                        s.start as usize,
                                        format!("/** @satisfies {{{load_ty}}} */ ("),
                                    ));
                                    insertions.push((s.end as usize, ")".to_string()));
                                }
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if insertions.is_empty() {
        return None;
    }

    insertions.sort_by_key(|(pos, _)| std::cmp::Reverse(*pos));
    let mut out = source.to_string();
    for (pos, text) in insertions {
        out.insert_str(pos, &text);
    }
    Some(out)
}

/// Mirrors upstream's load-event naming matrix. Server-side gets
/// `PageServerLoadEvent` / `LayoutServerLoadEvent`, client-side
/// `PageLoadEvent` / `LayoutLoadEvent`.
fn load_event_type(is_layout: bool, is_server: bool) -> String {
    let page_or_layout = if is_layout { "Layout" } else { "Page" };
    let server_infix = if is_server { "Server" } else { "" };
    format!("import('./$types.js').{page_or_layout}{server_infix}LoadEvent")
}

/// Bare `Load` type name (no `Event` suffix) for the non-function
/// `load` `satisfies` wrap — mirrors upstream's `type:'var'` branch,
/// which constrains the value against `(Page|Layout)(Server)?Load`
/// rather than the parameter-level `LoadEvent`.
fn load_satisfies_type(is_layout: bool, is_server: bool) -> String {
    let page_or_layout = if is_layout { "Layout" } else { "Page" };
    let server_infix = if is_server { "Server" } else { "" };
    format!("import('./$types.js').{page_or_layout}{server_infix}Load")
}

/// SvelteKit page-option exports with fixed value-union types. Names
/// match upstream's `addTypeToVariable` calls verbatim — any name not
/// in this list is left untouched (could be a user-defined export
/// that happens to be declared without a type).
fn page_option_type(name: &str) -> Option<&'static str> {
    match name {
        "prerender" => Some("boolean | 'auto'"),
        "ssr" => Some("boolean"),
        "csr" => Some("boolean"),
        "trailingSlash" => Some("'never' | 'always' | 'ignore'"),
        _ => None,
    }
}

/// Shared single-parameter-handler injection. Applies to both
/// `+server.ts` HTTP handlers and `+page.ts` `load` functions. Skips
/// multi-param and already-typed signatures (those don't match the
/// SvelteKit handler shape upstream injects against, so we leave
/// them alone rather than guess).
fn collect_handler_insert(
    func: &oxc_ast::ast::Function<'_>,
    event_type: &str,
    inject_response_return: bool,
    insertions: &mut Vec<(usize, String)>,
) {
    if func.params.items.len() != 1 {
        return;
    }
    let param = &func.params.items[0];
    if param.type_annotation.is_some() {
        return;
    }
    let insert_at = param.pattern.span().end as usize;
    insertions.push((insert_at, format!(": {event_type}")));

    // `+server.ts` HTTP handlers additionally get a return-type
    // constraint so a handler that returns a non-`Response` value is
    // flagged (TS2322). Mirrors upstream svelte2tsx
    // `helpers/sveltekit.ts::addTypeToFunction`, which — inside the same
    // `parameters.length === 1 && !hasTypeDefinition` gate we already
    // apply above — inserts a return annotation at the body-open brace
    // when the handler has no explicit return type: `Promise<Response>`
    // for an `async` handler, else `Response | Promise<Response>`.
    // `load` functions do NOT get a return type (upstream's `load`
    // branch injects only the parameter), hence the flag.
    if inject_response_return
        && func.return_type.is_none()
        && let Some(body) = func.body.as_ref()
    {
        let return_type = if func.r#async {
            "Promise<Response>"
        } else {
            "Response | Promise<Response>"
        };
        insertions.push((body.span.start as usize, format!(": {return_type} ")));
    }
}

/// JS twin of [`collect_handler_insert`]'s param annotation: insert a
/// `/** @param {<event_type>} <name> */ ` JSDoc block at
/// `insert_at` (the export statement start for declarations, the
/// function expression's own start for var-form initializers —
/// mirroring upstream `addJsDocParamToFunction`'s `node.getStart()`).
/// A binding-pattern param gets upstream's positional `arg0` stand-in
/// name; TypeScript matches it to the first parameter.
fn collect_js_param_insert(
    params: &oxc_ast::ast::FormalParameters<'_>,
    insert_at: usize,
    event_type: &str,
    insertions: &mut Vec<(usize, String)>,
) {
    if params.items.len() != 1 {
        return;
    }
    let param = &params.items[0];
    if param.type_annotation.is_some() {
        return;
    }
    let name = match &param.pattern {
        BindingPattern::BindingIdentifier(id) => id.name.as_str(),
        _ => "arg0",
    };
    insertions.push((insert_at, format!("/** @param {{{event_type}}} {name} */ ")));
}

/// JS twin of the `+server` handler's param + return annotation: one
/// `@type` JSDoc block typing the whole function —
/// `(arg0: RequestEvent) => Response | Promise<Response>` — inserted
/// before the export keyword (upstream `addTypeToFunction`'s JSDoc
/// branch builds exactly this shape; the async `Promise<Response>`
/// narrowing exists only on its TS path).
fn collect_js_fn_type_insert(
    func: &oxc_ast::ast::Function<'_>,
    insert_at: usize,
    fn_type: &str,
    insertions: &mut Vec<(usize, String)>,
) {
    if func.params.items.len() != 1 {
        return;
    }
    if func.params.items[0].type_annotation.is_some() {
        return;
    }
    insertions.push((insert_at, format!("/** @type {{{fn_type}}} */ ")));
}

/// Arrow-function twin of [`collect_handler_insert`]. Used for
/// `export const load = async ({…}) => {…}` form on `+page.ts` /
/// `+page.server.ts` / `+layout.ts` / `+layout.server.ts`. Same
/// "lone untyped param" heuristic as the function form — multi-arg
/// or already-typed arrows are left alone.
fn collect_arrow_handler_insert(
    arrow: &oxc_ast::ast::ArrowFunctionExpression<'_>,
    event_type: &str,
    insertions: &mut Vec<(usize, String)>,
) {
    if arrow.params.items.len() != 1 {
        return;
    }
    let param = &arrow.params.items[0];
    if param.type_annotation.is_some() {
        return;
    }
    let insert_at = param.pattern.span().end as usize;
    insertions.push((insert_at, format!(": {event_type}")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn server_path() -> PathBuf {
        PathBuf::from("src/routes/+server.ts")
    }
    fn page_path() -> PathBuf {
        PathBuf::from("src/routes/+page.ts")
    }
    fn layout_path() -> PathBuf {
        PathBuf::from("src/routes/+layout.ts")
    }
    fn page_server_path() -> PathBuf {
        PathBuf::from("src/routes/+page.server.ts")
    }
    fn layout_server_path() -> PathBuf {
        PathBuf::from("src/routes/+layout.server.ts")
    }

    #[test]
    fn classify_groups_inject_event_annotation() {
        let path = PathBuf::from("src/routes/(auth)/+page@(auth).ts");
        let source = "export async function load({ url }) { return {}; }";
        let got = inject(&path, source).expect("grouped route must inject");
        assert!(got.contains("PageLoadEvent"));
    }

    // +server.ts handler cases — existing coverage.

    #[test]
    fn injects_on_destructured_single_param() {
        let source =
            "export async function GET({ url }) {\n    return new Response(url.pathname);\n}";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains("({ url }: import('./$types.js').RequestEvent)"));
    }

    #[test]
    fn injects_on_identifier_param() {
        let source = "export function POST(event) { return new Response(''); }";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains("(event: import('./$types.js').RequestEvent)"));
    }

    #[test]
    fn injects_async_return_type_on_server_handler() {
        // Async handler → `Promise<Response>`, spliced before the body
        // brace so `return 42` fires TS2322 (upstream parity, #2966).
        let source = "export async function GET({ url }) { return 42; }";
        let got = inject(&server_path(), source).unwrap();
        assert!(
            got.contains("({ url }: import('./$types.js').RequestEvent) : Promise<Response> {"),
            "got: {got}"
        );
    }

    #[test]
    fn injects_sync_return_type_on_server_handler() {
        // Sync handler → `Response | Promise<Response>`.
        let source = "export function POST(event) { return new Response(''); }";
        let got = inject(&server_path(), source).unwrap();
        assert!(
            got.contains(
                "(event: import('./$types.js').RequestEvent) : Response | Promise<Response> {"
            ),
            "got: {got}"
        );
    }

    #[test]
    fn respects_explicit_server_handler_return_type() {
        // User-declared return type → we don't add a second one. The
        // param still gets `RequestEvent` (matches upstream's inner
        // `!fn.node.type` guard).
        let source =
            "export async function GET({ url }): Promise<Response> { return new Response(''); }";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains("({ url }: import('./$types.js').RequestEvent)"));
        // No injected return annotation duplicated onto the body brace.
        assert!(!got.contains(") : Promise<Response> {"), "got: {got}");
    }

    #[test]
    fn load_function_gets_no_return_type() {
        // `load` on a route file gets only the parameter annotation;
        // upstream's `load` branch never injects a return type.
        let source = "export async function load({ url }) { return { ok: true }; }";
        let got = inject(&page_path(), source).unwrap();
        assert!(got.contains("PageLoadEvent"));
        assert!(!got.contains("Promise<Response>"), "got: {got}");
        assert!(!got.contains("Response | Promise<Response>"), "got: {got}");
    }

    #[test]
    fn leaves_typed_param_alone() {
        let source = "export function GET(event: Foo) { return new Response(''); }";
        assert!(inject(&server_path(), source).is_none());
    }

    #[test]
    fn handles_multiple_handlers() {
        let source = "\
export async function GET({ url }) { return new Response(url.pathname); }
export async function POST({ request }) { return new Response(''); }
";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains("{ url }: import('./$types.js').RequestEvent"));
        assert!(got.contains("{ request }: import('./$types.js').RequestEvent"));
    }

    #[test]
    fn skips_non_handler_exports() {
        let source = "export function helper(x) { return x; }";
        assert!(inject(&server_path(), source).is_none());
    }

    #[test]
    fn skips_multi_param_handlers() {
        // Not a valid SvelteKit handler shape; don't guess.
        let source = "export function GET(a, b) { return new Response(''); }";
        assert!(inject(&server_path(), source).is_none());
    }

    #[test]
    fn non_kit_file_returns_none() {
        let source = "export async function GET({ url }) { return new Response(''); }";
        let helper_path = PathBuf::from("src/lib/helper.ts");
        assert!(inject(&helper_path, source).is_none());
    }

    #[test]
    fn preserves_bytes_outside_insertion() {
        let prefix = "// user comment\nexport async function GET({ url }) {";
        let suffix = "\n    return new Response(url.pathname);\n}\n";
        let source = format!("{prefix}{suffix}");
        let got = inject(&server_path(), &source).unwrap();
        assert!(got.starts_with("// user comment\n"));
        assert!(got.contains("return new Response(url.pathname);"));
    }

    // +page.ts load function — Page variant (client-side).

    #[test]
    fn page_load_gets_page_load_event() {
        let source = "export async function load({ params, fetch }) { return {}; }";
        let got = inject(&page_path(), source).unwrap();
        assert!(got.contains(": import('./$types.js').PageLoadEvent"));
    }

    #[test]
    fn layout_load_gets_layout_load_event() {
        let source = "export async function load({ params }) { return {}; }";
        let got = inject(&layout_path(), source).unwrap();
        assert!(got.contains(": import('./$types.js').LayoutLoadEvent"));
    }

    #[test]
    fn page_server_load_gets_page_server_load_event() {
        let source = "export async function load({ request }) { return {}; }";
        let got = inject(&page_server_path(), source).unwrap();
        assert!(got.contains(": import('./$types.js').PageServerLoadEvent"));
    }

    #[test]
    fn layout_server_load_gets_layout_server_load_event() {
        let source = "export async function load({ request }) { return {}; }";
        let got = inject(&layout_server_path(), source).unwrap();
        assert!(got.contains(": import('./$types.js').LayoutServerLoadEvent"));
    }

    #[test]
    fn non_load_function_in_page_is_ignored() {
        // Random user-defined helper — don't splice.
        let source = "export function helper({ x }) { return x; }";
        assert!(inject(&page_path(), source).is_none());
    }

    // Page-option variable-type injection.

    #[test]
    fn injects_ssr_boolean() {
        let source = "export const ssr = 'invalid';";
        let got = inject(&page_path(), source).unwrap();
        assert!(
            got.contains("export const ssr: boolean = 'invalid'"),
            "got: {got}"
        );
    }

    #[test]
    fn injects_csr_boolean() {
        let source = "export const csr = false;";
        let got = inject(&page_path(), source).unwrap();
        assert!(got.contains("csr: boolean = false"));
    }

    #[test]
    fn injects_prerender_union() {
        let source = "export const prerender = 'auto';";
        let got = inject(&page_path(), source).unwrap();
        assert!(got.contains("prerender: boolean | 'auto' = 'auto'"));
    }

    #[test]
    fn injects_trailing_slash_union() {
        let source = "export const trailingSlash = 'always';";
        let got = inject(&page_path(), source).unwrap();
        assert!(got.contains("trailingSlash: 'never' | 'always' | 'ignore' = 'always'"));
    }

    #[test]
    fn leaves_typed_page_options_alone() {
        let source = "export const ssr: boolean = true;";
        assert!(inject(&page_path(), source).is_none());
    }

    #[test]
    fn skips_unknown_page_consts() {
        // User-defined export that happens to be a bare const.
        let source = "export const myCustomThing = 42;";
        assert!(inject(&page_path(), source).is_none());
    }

    #[test]
    fn layout_also_accepts_page_options() {
        let source = "export const ssr = true;";
        let got = inject(&layout_path(), source).unwrap();
        assert!(got.contains("ssr: boolean = true"));
    }

    #[test]
    fn server_endpoint_ignores_page_options() {
        // +server.ts doesn't support `ssr` etc. — our ServerEndpoint
        // branch only looks at HTTP handlers, so page-options are
        // untouched even if the user happens to write one.
        let source = "export const ssr = true;";
        assert!(inject(&server_path(), source).is_none());
    }

    // .js route files — JSDoc-form injections (upstream's !isTsFile
    // branches of upsertKitFile).

    fn page_js_path() -> PathBuf {
        PathBuf::from("src/routes/+page.js")
    }
    fn server_js_path() -> PathBuf {
        PathBuf::from("src/routes/+server.js")
    }

    #[test]
    fn js_load_function_gets_jsdoc_param() {
        let source = "export function load({ params }) {\n\treturn {};\n}\n";
        let got = inject(&page_js_path(), source).unwrap();
        assert!(
            got.starts_with(
                "/** @param {import('./$types.js').PageLoadEvent} arg0 */ export function load"
            ),
            "got: {got}"
        );
    }

    #[test]
    fn js_load_identifier_param_keeps_its_name() {
        let source = "export function load(event) { return {}; }";
        let got = inject(&page_js_path(), source).unwrap();
        assert!(
            got.contains("/** @param {import('./$types.js').PageLoadEvent} event */ "),
            "got: {got}"
        );
    }

    #[test]
    fn js_arrow_load_gets_jsdoc_param_before_arrow() {
        let source = "export const load = async ({ fetch }) => ({});";
        let got = inject(&page_js_path(), source).unwrap();
        assert!(
            got.contains(
                "export const load = /** @param {import('./$types.js').PageLoadEvent} arg0 */ async ({ fetch }) =>"
            ),
            "got: {got}"
        );
    }

    #[test]
    fn js_page_option_gets_jsdoc_cast() {
        let source = "export const prerender = 'sometimes';";
        let got = inject(&page_js_path(), source).unwrap();
        assert_eq!(
            got,
            "export const prerender = /** @type {boolean | 'auto'} */ ('sometimes');"
        );
    }

    #[test]
    fn js_non_function_load_gets_jsdoc_satisfies() {
        let source = "export const load = loader;";
        let got = inject(&page_js_path(), source).unwrap();
        assert_eq!(
            got,
            "export const load = /** @satisfies {import('./$types.js').PageLoad} */ (loader);"
        );
    }

    #[test]
    fn js_server_handler_gets_whole_fn_jsdoc_type() {
        let source = "export async function GET({ url }) { return 42; }";
        let got = inject(&server_js_path(), source).unwrap();
        assert!(
            got.starts_with(
                "/** @type {(arg0: import('./$types.js').RequestEvent) => Response | Promise<Response>} */ export async function GET"
            ),
            "got: {got}"
        );
    }

    #[test]
    fn js_user_jsdoc_typed_export_is_left_alone() {
        // Upstream's hasTypeDefinition: an existing @type / @param /
        // @satisfies JSDoc on the export means the user typed it.
        for source in [
            "/** @param {import('./$types').PageLoadEvent} event */\nexport function load(event) { return {}; }",
            "/** @type {import('./$types').PageLoad} */\nexport const load = () => ({});",
            "/** @type {boolean} */\nexport const ssr = true;",
        ] {
            assert!(
                inject(&page_js_path(), source).is_none(),
                "should not double-type: {source}"
            );
        }
    }

    #[test]
    fn js_injections_add_no_lines() {
        // Kit overlays map positions via an identity line map — a JS
        // injection must never add or remove lines.
        let source = "export function load({ params }) {\n\tvoid params.nope;\n\treturn {};\n}\n";
        let got = inject(&page_js_path(), source).unwrap();
        assert_eq!(
            source.matches('\n').count(),
            got.matches('\n').count(),
            "JSDoc injection changed the line count"
        );
    }
}
