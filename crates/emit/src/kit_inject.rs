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
//! - Hooks files (`src/hooks.server.ts` / `hooks.client.ts` /
//!   `hooks.ts`, and their configured or directory forms) — each
//!   recognised hook export gets its handler type projected onto both
//!   the parameter (`Parameters<T>[0]`) and the return (`ReturnType<T>`).
//!   Which module `T` comes from depends on the installed SvelteKit:
//!   `@sveltejs/kit` before 3, `@sveltejs/kit/hooks` from 3 on.
//! - Param matchers (`src/params/*.ts`) — `match` gets the one
//!   concrete pair upstream uses, `string` in and `boolean` out.
//!
//! `.js` route files receive the same injections in JSDoc form
//! (mirrors upstream `upsertKitFile`'s `isTsFile` split): `@param`
//! blocks for `load` handlers, `@type` casts for page options, a
//! whole-function `@type` for `+server.js` handlers, and `@satisfies`
//! casts for non-function `load` values.
//!
//! `actions` gets a `satisfies import('./$types.js').Actions` trailer,
//! a parameterless `entries` its `ReturnType<EntryGenerator>` return
//! annotation, and `+server` files receive the page-option and
//! `load` / `entries` typing as well as their HTTP handlers — every
//! export upstream's `upsertKitFile` annotates on a kit route file.

use oxc_allocator::Allocator;
use oxc_ast::ast::{BindingPattern, Declaration, ExportDefaultDeclarationKind, Statement};
use oxc_span::GetSpan;
use std::path::Path;
use svn_core::sveltekit::{HooksScope, KitFilesSettings, KitRole, ScriptLang, classify};
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
    /// `+server.ts` — HTTP handlers get `RequestEvent`; page options,
    /// `load` and `entries` are typed as on a server page file.
    ServerEndpoint,
    /// `+page.ts`, `+layout.ts`, `+page.server.ts`, `+layout.server.ts`.
    /// `load` gets a type-matrix-derived `LoadEvent`; page-option
    /// consts get their fixed-union types. Sub-classification feeds
    /// the load-event name computation.
    Route { is_layout: bool, is_server: bool },
    /// `src/hooks.server.ts` / `src/hooks.client.ts` / `src/hooks.ts`
    /// (and the `<hooks-path>/index.ts` directory form). Each named
    /// hook export gets its handler type projected onto the parameter
    /// and the return.
    Hooks { scope: HooksScope },
    /// `src/params/<matcher>.ts`. Only `match` is typed: `string` in,
    /// `boolean` out.
    Params,
}

/// The hook exports each scope recognises, paired with the type name
/// to project. Mirrors upstream svelte2tsx's `upsertKitServerHooksFile`
/// / `upsertKitClientHooksFile` / `upsertKitUniversalHooksFile` — the
/// set is closed, and an export outside it is left exactly as written.
fn hook_type_name(scope: HooksScope, export_name: &str) -> Option<&'static str> {
    match (scope, export_name) {
        (HooksScope::Server, "handleError") => Some("HandleServerError"),
        (HooksScope::Server, "handle") => Some("Handle"),
        (HooksScope::Server, "handleFetch") => Some("HandleFetch"),
        (HooksScope::Client, "handleError") => Some("HandleClientError"),
        (HooksScope::Universal, "reroute") => Some("Reroute"),
        _ => None,
    }
}

/// Everything the inject pass needs beyond the file itself.
///
/// `settings` matters because hooks and param-matcher paths are
/// user-configurable (`kit.files.hooks` / `kit.files.params`); route
/// files are pure basename shapes and ignore it.
#[derive(Debug, Clone, Copy)]
pub struct KitInjectOptions<'a> {
    pub settings: &'a KitFilesSettings,
    /// Module the hook types are imported from — `@sveltejs/kit` before
    /// SvelteKit 3, `@sveltejs/kit/hooks` from 3 on. Resolve it with
    /// `svn_core::sveltekit::hooks_types_module`.
    pub hooks_types_module: &'a str,
}

/// Classify `path` for kit_inject's purposes. Returns `None` for
/// route components (they go through emit's overlay pipeline instead)
/// and plain user files.
///
/// The second tuple element is `true` for `.ts` sources; `.js` route
/// files get the same injections in JSDoc form (mirrors upstream
/// `upsertKitFile`'s `isTsFile` split).
fn kit_file_kind(path: &Path, settings: &KitFilesSettings) -> Option<(KitFileKind, bool)> {
    let kit = classify(path, settings)?;
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
        KitRole::Hooks { scope } => Some((KitFileKind::Hooks { scope }, is_ts)),
        KitRole::Params => Some((KitFileKind::Params, is_ts)),
        // Route components go through emit's overlay pipeline, not
        // this pass — return None so the caller skips them.
        KitRole::RouteComponent { .. } => None,
    }
}

/// The tags of the JSDoc block TypeScript attaches to a node, as
/// `(tag name, tag text)` pairs — what `ts.getJSDocTags` reads.
///
/// A node's JSDoc comes from the `/** … */` comments between the end of
/// the token before it (`trivia_start`) and its own start. TypeScript
/// counts such a comment only after a line break outside any comment
/// (or at the very start of the file), except for the node kinds whose
/// same-line comments it reads too (`same_line`: variable declarations,
/// parameters, function expressions and arrows). Only the LAST block
/// contributes tags; earlier ones are ignored.
fn attached_jsdoc_tags<'s>(
    comments: &[oxc_ast::Comment],
    source: &'s str,
    trivia_start: usize,
    node_start: usize,
    same_line: bool,
) -> Vec<(&'s str, &'s str)> {
    let mut collecting = same_line || trivia_start == 0;
    let mut gap_start = trivia_start;
    let mut last: Option<&'s str> = None;
    for comment in comments {
        let (start, end) = (comment.span.start as usize, comment.span.end as usize);
        if start < trivia_start || end > node_start {
            continue;
        }
        collecting |= source
            .get(gap_start..start)
            .is_some_and(|gap| gap.contains(['\n', '\r']));
        gap_start = end;
        let Some(text) = source.get(start..end) else {
            continue;
        };
        if collecting && text.starts_with("/**") && !text.starts_with("/**/") {
            last = Some(text);
        }
    }
    last.map(jsdoc_block_tags).unwrap_or_default()
}

/// Split a `/** … */` block into its tags. A tag starts at an `@` that
/// opens a line (after the optional `*` margin) or follows whitespace,
/// and runs until the next tag.
fn jsdoc_block_tags(block: &str) -> Vec<(&str, &str)> {
    let body = block
        .strip_prefix("/**")
        .and_then(|b| b.strip_suffix("*/"))
        .unwrap_or("");
    let bytes = body.as_bytes();
    let mut starts: Vec<(usize, usize)> = Vec::new();
    for (i, &b) in bytes.iter().enumerate() {
        if b != b'@' {
            continue;
        }
        let opens = i == 0 || matches!(bytes[i - 1], b' ' | b'\t' | b'\n' | b'\r' | b'*');
        let name_len = body[i + 1..]
            .bytes()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_')
            .count();
        if opens && name_len > 0 {
            starts.push((i, i + 1 + name_len));
        }
    }
    starts
        .iter()
        .enumerate()
        .map(|(k, &(_, name_end))| {
            let text_end = starts.get(k + 1).map_or(body.len(), |&(next, _)| next);
            (&body[starts[k].0 + 1..name_end], &body[name_end..text_end])
        })
        .collect()
}

fn has_type_tag(tags: &[(&str, &str)]) -> bool {
    tags.iter().any(|(name, _)| *name == "type")
}

fn has_satisfies_tag(tags: &[(&str, &str)]) -> bool {
    tags.iter().any(|(name, _)| *name == "satisfies")
}

/// `ts.getJSDocParameterTags(first)` is non-empty: a `@param` (or its
/// `@arg` / `@argument` aliases) naming the parameter, or — for a
/// destructured one, which TypeScript matches by position — any.
fn has_param_tag(tags: &[(&str, &str)], first: &LoneParam<'_>) -> bool {
    tags.iter()
        .filter(|(name, _)| matches!(*name, "param" | "arg" | "argument"))
        .any(|(_, text)| match first.name {
            None => true,
            Some(name) => param_tag_name(text) == Some(name),
        })
}

/// The parameter name a `@param` tag's text documents:
/// `{Type} name`, `name`, `[name]` or `[name=default]`.
fn param_tag_name(text: &str) -> Option<&str> {
    let mut rest = text.trim_start();
    if rest.starts_with('{') {
        let mut depth = 0usize;
        let mut end = rest.len();
        for (i, c) in rest.char_indices() {
            match c {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        end = i + 1;
                        break;
                    }
                }
                _ => {}
            }
        }
        rest = rest[end..].trim_start();
    }
    let rest = rest.strip_prefix('[').unwrap_or(rest).trim_start();
    let len = rest
        .bytes()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == b'_' || *c == b'$')
        .count();
    (len > 0).then(|| &rest[..len])
}

/// The first parameter of a function, as upstream's
/// `node.parameters[0]` sees it (a rest parameter counts).
struct LoneParam<'a> {
    /// End of the whole parameter, default value included — where
    /// upstream splices the parameter annotation.
    end: usize,
    typed: bool,
    /// The bound name when the parameter is a plain identifier.
    name: Option<&'a str>,
}

/// The parameter count and first parameter of `params`.
fn first_param<'a>(
    params: &'a oxc_ast::ast::FormalParameters<'a>,
) -> (usize, Option<LoneParam<'a>>) {
    let count = params.items.len() + usize::from(params.rest.is_some());
    let first = match (params.items.first(), params.rest.as_ref()) {
        (Some(p), _) => Some(LoneParam {
            end: p.span.end as usize,
            typed: p.type_annotation.is_some(),
            name: match &p.pattern {
                BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
                _ => None,
            },
        }),
        (None, Some(rest)) => Some(LoneParam {
            end: rest.span.end as usize,
            typed: rest.type_annotation.is_some(),
            name: match &rest.rest.argument {
                BindingPattern::BindingIdentifier(id) => Some(id.name.as_str()),
                _ => None,
            },
        }),
        (None, None) => None,
    };
    (count, first)
}

/// The single parameter of a one-parameter function, or `None` when
/// the function takes more or fewer — upstream only annotates
/// `parameters.length === 1` handlers.
fn lone_param<'a>(params: &'a oxc_ast::ast::FormalParameters<'a>) -> Option<LoneParam<'a>> {
    match first_param(params) {
        (1, first) => first,
        _ => None,
    }
}

/// Upstream `hasTypedParameter`: the first parameter carries a TS type,
/// or — in a JS file — the function's JSDoc has a `@type` tag or a
/// `@param` for that parameter.
fn has_typed_parameter(
    params: &oxc_ast::ast::FormalParameters<'_>,
    is_ts: bool,
    jsdoc: &[(&str, &str)],
) -> bool {
    let first = first_param(params).1;
    if first.as_ref().is_some_and(|p| p.typed) {
        return true;
    }
    !is_ts && (has_type_tag(jsdoc) || first.is_some_and(|p| has_param_tag(jsdoc, &p)))
}

/// A function value an exported `const` holds, as upstream's
/// `findExports` recognises it: an arrow or function expression, bare
/// or in parentheses. `satisfies`-wrapped functions count too, but are
/// always typed, so they're reported as `None` here and caught by the
/// declaration's own type check.
enum FnValue<'a> {
    Arrow(&'a oxc_ast::ast::ArrowFunctionExpression<'a>),
    Function(&'a oxc_ast::ast::Function<'a>),
}

impl<'a> FnValue<'a> {
    fn of(init: &'a oxc_ast::ast::Expression<'a>) -> Option<(Self, bool)> {
        use oxc_ast::ast::Expression;
        let (inner, parenthesized) = match init {
            Expression::ParenthesizedExpression(p) => (&p.expression, true),
            other => (other, false),
        };
        match inner {
            Expression::ArrowFunctionExpression(a) => Some((Self::Arrow(a), parenthesized)),
            Expression::FunctionExpression(f) => Some((Self::Function(f), parenthesized)),
            _ => None,
        }
    }

    fn params(&self) -> &'a oxc_ast::ast::FormalParameters<'a> {
        match self {
            Self::Arrow(a) => &a.params,
            Self::Function(f) => &f.params,
        }
    }

    fn start(&self) -> usize {
        match self {
            Self::Arrow(a) => a.span.start as usize,
            Self::Function(f) => f.span.start as usize,
        }
    }
}

/// Upstream `findExports`' `hasTypeDefinition` for an exported
/// single-declarator `const`, plus `hasTypedParameter` when its value
/// is a function.
///
/// `stmt_tags` is the export statement's JSDoc. In a JS file the
/// declaration also reads JSDoc written on itself and on a function
/// value, and a function value reads the statement's JSDoc unless it
/// is parenthesized (a parenthesized function is a JSDoc cast target,
/// not the declaration's own value).
fn var_export_is_typed(
    declarator: &oxc_ast::ast::VariableDeclarator<'_>,
    decl_trivia_start: usize,
    is_ts: bool,
    stmt_tags: &[(&str, &str)],
    comments: &[oxc_ast::Comment],
    source: &str,
) -> bool {
    use oxc_ast::ast::Expression;
    if declarator.type_annotation.is_some() {
        return true;
    }
    let init = declarator.init.as_ref();
    if matches!(init, Some(Expression::TSSatisfiesExpression(_))) {
        return true;
    }
    let fn_value = init.and_then(FnValue::of);
    if is_ts {
        return fn_value.is_some_and(|(f, _)| has_typed_parameter(f.params(), true, &[]));
    }
    let own_tags = attached_jsdoc_tags(
        comments,
        source,
        decl_trivia_start,
        declarator.span.start as usize,
        true,
    );
    // The token before a function value is the `=`.
    let fn_tags = fn_value.as_ref().map(|(f, _)| {
        let eq = source[..f.start()].rfind('=').map_or(f.start(), |i| i + 1);
        attached_jsdoc_tags(comments, source, eq, f.start(), true)
    });
    let unparenthesized_fn_tags = match &fn_value {
        Some((_, false)) => fn_tags.as_deref().unwrap_or(&[]),
        _ => &[],
    };
    let declaration_tags: Vec<(&str, &str)> = stmt_tags
        .iter()
        .chain(own_tags.iter())
        .chain(unparenthesized_fn_tags.iter())
        .copied()
        .collect();
    if has_type_tag(&declaration_tags) || has_satisfies_tag(&declaration_tags) {
        return true;
    }
    let Some((f, parenthesized)) = fn_value else {
        return false;
    };
    let mut function_tags: Vec<(&str, &str)> = fn_tags.unwrap_or_default();
    if !parenthesized {
        function_tags.extend(stmt_tags.iter().copied());
    }
    has_typed_parameter(f.params(), false, &function_tags)
}

/// The declaration an `export` statement carries, when it's one
/// upstream's `findExports` reads.
enum ExportedDecl<'a> {
    Function(&'a oxc_ast::ast::Function<'a>),
    Variable(&'a oxc_ast::ast::VariableDeclaration<'a>),
}

/// A kit file's overlay: the user's source with type annotations
/// spliced in, plus where they went.
pub struct Injected {
    /// The overlay text.
    pub text: String,
    /// Every splice, as `(byte offset in `text`, byte length)`, in
    /// ascending offset order. Diagnostic mapping subtracts these to
    /// recover the user's column — without them a diagnostic reported
    /// against an annotated line points past where the user typed.
    pub insertions: Vec<(usize, usize)>,
}

/// Returns the modified source with injected type annotations, or
/// `None` if no injections were needed (no handlers matched OR all
/// handlers already carry explicit types).
///
/// The returned string preserves the original source's byte layout
/// except at the insertion points — every insertion is purely
/// additive and none contains a newline, so line numbers are identical
/// on both sides and only columns after a splice move.
pub fn inject(path: &Path, source: &str, opts: &KitInjectOptions<'_>) -> Option<Injected> {
    let (kind, is_ts) = kit_file_kind(path, opts.settings)?;

    // The type pair for a named export of a hooks / params file, or
    // `None` when the export isn't one this file's role recognises.
    let handler_types = |name: &str| -> Option<HandlerTypes> {
        match kind {
            KitFileKind::Hooks { scope } => hook_type_name(scope, name).map(|ty| {
                let module = opts.hooks_types_module;
                HandlerTypes::Projected {
                    handler: format!("import('{module}').{ty}"),
                }
            }),
            KitFileKind::Params if name == "match" => Some(HandlerTypes::Concrete {
                param: "string",
                ret: "boolean",
            }),
            _ => None,
        }
    };

    let alloc = Allocator::default();
    let parsed = parse_script_body(&alloc, source, ParserScriptLang::Ts);

    let comments = &parsed.program.comments;
    let body = &parsed.program.body;
    let mut insertions: Vec<(usize, String)> = Vec::new();
    for (idx, stmt) in body.iter().enumerate() {
        // Upstream's `findExports` registers every statement whose first
        // modifier is `export`: `export function`, `export default
        // function` and single-declarator `export const`. A bare
        // specifier list (`export { handle }`) declares nothing.
        let (export_start, declaration) = match stmt {
            Statement::ExportDeclaration(export) => match &export.declaration {
                Declaration::FunctionDeclaration(func) => {
                    (export.span.start, ExportedDecl::Function(func))
                }
                Declaration::VariableDeclaration(var_decl) => {
                    (export.span.start, ExportedDecl::Variable(var_decl))
                }
                _ => continue,
            },
            Statement::ExportDefaultDeclaration(export) => match &export.declaration {
                ExportDefaultDeclarationKind::FunctionDeclaration(func) => {
                    (export.span.start, ExportedDecl::Function(func))
                }
                _ => continue,
            },
            _ => continue,
        };
        let export_start = export_start as usize;
        let trivia_start = idx
            .checked_sub(1)
            .map_or(0, |i| body[i].span().end as usize);
        // A JS export's own JSDoc — what `ts.getJSDocType` and friends
        // read for upstream's `hasTypeDefinition`.
        let stmt_tags = if is_ts {
            Vec::new()
        } else {
            attached_jsdoc_tags(comments, source, trivia_start, export_start, false)
        };

        match declaration {
            ExportedDecl::Function(func) => {
                let Some(name) = func.id.as_ref().map(|id| id.name.as_str()) else {
                    continue;
                };
                if has_typed_parameter(&func.params, is_ts, &stmt_tags) {
                    continue;
                }
                match &kind {
                    KitFileKind::ServerEndpoint if SERVER_HANDLER_NAMES.contains(&name) => {
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
                                export_start,
                                "(arg0: import('./$types.js').RequestEvent) => Response | Promise<Response>",
                                &mut insertions,
                            );
                        }
                    }
                    KitFileKind::Hooks { .. } | KitFileKind::Params => {
                        let Some(types) = handler_types(name) else {
                            continue;
                        };
                        if is_ts {
                            collect_projected_handler_insert(
                                &func.params,
                                func.return_type
                                    .is_none()
                                    .then(|| func.body.as_ref().map(|b| b.span.start as usize))
                                    .flatten(),
                                &types,
                                &mut insertions,
                            );
                        } else {
                            collect_js_fn_type_insert(
                                func,
                                export_start,
                                &types.jsdoc_type(),
                                &mut insertions,
                            );
                        }
                    }
                    KitFileKind::Route { .. } | KitFileKind::ServerEndpoint => {
                        let (is_layout, is_server) = route_flags(&kind);
                        // A parameterless `entries` with no declared
                        // return type: upstream annotates the return
                        // as `ReturnType<EntryGenerator>` (TS: at the
                        // body's `{`; JS: a `@type` on the function),
                        // on every kit route file but a layout.
                        if name == "entries" {
                            if !is_layout
                                && func.params.items.is_empty()
                                && func.return_type.is_none()
                                && let Some(body) = func.body.as_ref()
                            {
                                if is_ts {
                                    insertions.push((
                                        body.span.start as usize,
                                        format!(": ReturnType<{ENTRY_GENERATOR}> "),
                                    ));
                                } else {
                                    insertions.push((
                                        export_start,
                                        format!("/** @type {{{ENTRY_GENERATOR}}} */ "),
                                    ));
                                }
                            }
                            continue;
                        }
                        if name != "load" {
                            continue;
                        }
                        let event_type = load_event_type(is_layout, is_server);
                        if is_ts {
                            collect_handler_insert(func, &event_type, false, &mut insertions);
                        } else {
                            // JS: `@param` on the (lone) event arg —
                            // upstream's addJsDocParamToFunction. The
                            // return stays untyped, same as the TS
                            // branch.
                            collect_js_param_insert(
                                &func.params,
                                export_start,
                                &event_type,
                                &mut insertions,
                            );
                        }
                    }
                }
            }
            ExportedDecl::Variable(var_decl) => {
                // Upstream's findExports only registers single-declarator
                // export const statements (declarations.length === 1); a
                // multi-declarator list is ignored entirely, so skip it
                // here to avoid injecting types upstream never would.
                if var_decl.declarations.len() != 1 {
                    continue;
                }
                let decl_trivia_start = var_decl.span.start as usize + var_decl.kind.as_str().len();
                for declarator in &var_decl.declarations {
                    if declarator.init.is_none() {
                        continue;
                    }
                    let BindingPattern::BindingIdentifier(id) = &declarator.id else {
                        continue;
                    };
                    if var_export_is_typed(
                        declarator,
                        decl_trivia_start,
                        is_ts,
                        &stmt_tags,
                        comments,
                        source,
                    ) {
                        continue;
                    }

                    // Hooks and param matchers export function values
                    // only, so they're fully handled here — nothing
                    // below this point applies to them.
                    if matches!(kind, KitFileKind::Hooks { .. } | KitFileKind::Params) {
                        // An annotated variable (`export const handle:
                        // Handle = …`) is upstream's `hasTypeDefinition`
                        // — the user has taken responsibility for the
                        // signature.
                        if declarator.type_annotation.is_some() {
                            continue;
                        }
                        let Some(types) = handler_types(id.name.as_str()) else {
                            continue;
                        };
                        if let Some(init) = declarator.init.as_ref() {
                            collect_fn_value_insert(init, source, is_ts, &types, &mut insertions);
                        }
                        continue;
                    }

                    // `export const GET = (event) => …` on `+server`:
                    // upstream's `findExports` registers the arrow /
                    // function-expression value as a function and
                    // `insertApiMethod` types it like the declaration
                    // form.
                    if matches!(kind, KitFileKind::ServerEndpoint)
                        && SERVER_HANDLER_NAMES.contains(&id.name.as_str())
                    {
                        use oxc_ast::ast::Expression;
                        if declarator.type_annotation.is_some() {
                            continue;
                        }
                        let Some(init) = declarator.init.as_ref() else {
                            continue;
                        };
                        let is_async = match init {
                            Expression::ArrowFunctionExpression(a) => a.r#async,
                            Expression::FunctionExpression(f) => f.r#async,
                            _ => false,
                        };
                        let types = HandlerTypes::Concrete {
                            param: "import('./$types.js').RequestEvent",
                            ret: if is_async {
                                "Promise<Response>"
                            } else {
                                "Response | Promise<Response>"
                            },
                        };
                        collect_fn_value_insert(init, source, is_ts, &types, &mut insertions);
                        continue;
                    }

                    let (is_layout, is_server) = route_flags(&kind);
                    let is_layout = &is_layout;
                    let is_server = &is_server;

                    // `export const actions = { … }` — upstream's
                    // `satisfies import('./$types.js').Actions` trailer
                    // (TS) or `@satisfies` cast (JS), unless the user
                    // annotated the variable.
                    if id.name.as_str() == "actions" {
                        if declarator.type_annotation.is_none()
                            && let Some(init) = declarator.init.as_ref()
                        {
                            let sp = init.span();
                            if is_ts {
                                insertions.push((sp.end as usize, format!(" satisfies {ACTIONS}")));
                            } else {
                                insertions.push((
                                    sp.start as usize,
                                    format!("/** @satisfies {{{ACTIONS}}} */ ("),
                                ));
                                insertions.push((sp.end as usize, ")".to_string()));
                            }
                        }
                        continue;
                    }

                    // `export const entries = () => …` — the value form
                    // of the `entries` generator, typed like the
                    // declaration form.
                    if id.name.as_str() == "entries" {
                        use oxc_ast::ast::Expression;
                        if !*is_layout
                            && declarator.type_annotation.is_none()
                            && let Some(init) = declarator.init.as_ref()
                        {
                            match init {
                                Expression::ArrowFunctionExpression(arrow)
                                    if arrow.params.items.is_empty()
                                        && arrow.return_type.is_none() =>
                                {
                                    if is_ts {
                                        if let Some(at) = arrow_token_pos(
                                            source,
                                            arrow.params.span.end as usize,
                                            arrow.body.span().start as usize,
                                        ) {
                                            insertions.push((
                                                at,
                                                format!(": ReturnType<{ENTRY_GENERATOR}> "),
                                            ));
                                        }
                                    } else {
                                        insertions.push((
                                            arrow.span.start as usize,
                                            format!("/** @type {{{ENTRY_GENERATOR}}} */ "),
                                        ));
                                    }
                                }
                                Expression::FunctionExpression(func)
                                    if func.params.items.is_empty()
                                        && func.return_type.is_none() =>
                                {
                                    if is_ts {
                                        if let Some(body) = func.body.as_ref() {
                                            insertions.push((
                                                body.span.start as usize,
                                                format!(": ReturnType<{ENTRY_GENERATOR}> "),
                                            ));
                                        }
                                    } else {
                                        insertions.push((
                                            func.span.start as usize,
                                            format!("/** @type {{{ENTRY_GENERATOR}}} */ "),
                                        ));
                                    }
                                }
                                _ => {}
                            }
                        }
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
        }
    }

    if insertions.is_empty() {
        return None;
    }

    // Splice back-to-front so each insertion's source offset is still
    // valid when it happens, then walk forward once to record where
    // each landed in the OUTPUT — which is the source offset shifted by
    // everything spliced ahead of it.
    insertions.sort_by_key(|(pos, _)| *pos);
    let mut out = String::with_capacity(
        source.len() + insertions.iter().map(|(_, t)| t.len()).sum::<usize>(),
    );
    let mut placed = Vec::with_capacity(insertions.len());
    let mut cursor = 0usize;
    for (pos, text) in &insertions {
        out.push_str(&source[cursor..*pos]);
        placed.push((out.len(), text.len()));
        out.push_str(text);
        cursor = *pos;
    }
    out.push_str(&source[cursor..]);

    Some(Injected {
        text: out,
        insertions: placed,
    })
}

const ACTIONS: &str = "import('./$types.js').Actions";
const ENTRY_GENERATOR: &str = "import('./$types.js').EntryGenerator";

/// `(is_layout, is_server)` for the route-like typing shared by route
/// scripts and `+server` files; upstream derives both from the file's
/// basename (`layout` / `server` substrings), so `+server` counts as a
/// server page file.
fn route_flags(kind: &KitFileKind) -> (bool, bool) {
    match kind {
        KitFileKind::Route {
            is_layout,
            is_server,
        } => (*is_layout, *is_server),
        KitFileKind::ServerEndpoint => (false, true),
        KitFileKind::Hooks { .. } | KitFileKind::Params => (false, false),
    }
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
    let Some(param) = lone_param(&func.params).filter(|p| !p.typed) else {
        return;
    };
    insertions.push((param.end, format!(": {event_type}")));

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
    let Some(param) = lone_param(params).filter(|p| !p.typed) else {
        return;
    };
    let name = param.name.unwrap_or("arg0");
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
    if lone_param(&func.params).is_none_or(|p| p.typed) {
        return;
    }
    insertions.push((insert_at, format!("/** @type {{{fn_type}}} */ ")));
}

/// The type pair a hooks / param-matcher export gets. Upstream's
/// `addTypeToFunction` takes an optional concrete return type and
/// branches on it: with one, the parameter and return are annotated
/// with the given types directly; without one, both are *projected*
/// off a single handler type via `Parameters<T>[0]` and `ReturnType<T>`.
///
/// Hooks take the projecting form (one handler type each); a param
/// matcher takes the concrete form (`string` in, `boolean` out).
enum HandlerTypes {
    Projected {
        handler: String,
    },
    Concrete {
        param: &'static str,
        ret: &'static str,
    },
}

impl HandlerTypes {
    fn param_annotation(&self) -> String {
        match self {
            Self::Projected { handler } => format!(": Parameters<{handler}>[0]"),
            Self::Concrete { param, .. } => format!(": {param}"),
        }
    }

    fn return_annotation(&self) -> String {
        match self {
            Self::Projected { handler } => format!(": ReturnType<{handler}> "),
            Self::Concrete { ret, .. } => format!(": {ret} "),
        }
    }

    /// The JSDoc `@type` for the `.js` form. Upstream collapses the
    /// pair into one function type when a concrete return exists, and
    /// otherwise names the handler type directly.
    fn jsdoc_type(&self) -> String {
        match self {
            Self::Projected { handler } => handler.clone(),
            Self::Concrete { param, ret } => format!("(arg0: {param}) => {ret}"),
        }
    }
}

/// Splice a handler type onto a lone untyped parameter and, when the
/// function has no return type of its own, onto its return position.
///
/// `return_insert_at` is where the return annotation goes — the body's
/// opening brace for a function, the `=>` token for an arrow — or
/// `None` when the function already declares a return type (or has no
/// body, e.g. an overload signature). Mirrors upstream svelte2tsx
/// `helpers/sveltekit.ts::addTypeToFunction`.
fn collect_projected_handler_insert(
    params: &oxc_ast::ast::FormalParameters<'_>,
    return_insert_at: Option<usize>,
    types: &HandlerTypes,
    insertions: &mut Vec<(usize, String)>,
) {
    let Some(param) = lone_param(params).filter(|p| !p.typed) else {
        return;
    };
    insertions.push((param.end, types.param_annotation()));
    if let Some(pos) = return_insert_at {
        insertions.push((pos, types.return_annotation()));
    }
}

/// Splice a handler type onto a `export const <name> = <fn>` form,
/// where `<fn>` is an arrow or a function expression. Any other
/// initializer — an identifier, a `satisfies` wrapper, a call — is
/// left alone: upstream's `findExports` only registers the two
/// function forms as typeable, and a `satisfies` wrapper is
/// user-supplied typing besides.
fn collect_fn_value_insert(
    init: &oxc_ast::ast::Expression<'_>,
    source: &str,
    is_ts: bool,
    types: &HandlerTypes,
    insertions: &mut Vec<(usize, String)>,
) {
    use oxc_ast::ast::Expression;
    match init {
        Expression::ArrowFunctionExpression(arrow) => {
            if is_ts {
                let return_at = arrow.return_type.is_none().then(|| {
                    arrow_token_pos(
                        source,
                        arrow.params.span.end as usize,
                        arrow.body.span().start as usize,
                    )
                });
                collect_projected_handler_insert(
                    &arrow.params,
                    return_at.flatten(),
                    types,
                    insertions,
                );
            } else {
                collect_js_value_fn_type_insert(
                    &arrow.params,
                    arrow.span.start as usize,
                    types,
                    insertions,
                );
            }
        }
        Expression::FunctionExpression(func) => {
            if is_ts {
                collect_projected_handler_insert(
                    &func.params,
                    func.return_type
                        .is_none()
                        .then(|| func.body.as_ref().map(|b| b.span.start as usize))
                        .flatten(),
                    types,
                    insertions,
                );
            } else {
                collect_js_value_fn_type_insert(
                    &func.params,
                    func.span.start as usize,
                    types,
                    insertions,
                );
            }
        }
        _ => {}
    }
}

/// JS twin of [`collect_fn_value_insert`]: one `@type` JSDoc block in
/// front of the function value, mirroring upstream
/// `addJsDocTypeToFunction`, which anchors on the function node's own
/// start rather than the export statement's.
fn collect_js_value_fn_type_insert(
    params: &oxc_ast::ast::FormalParameters<'_>,
    insert_at: usize,
    types: &HandlerTypes,
    insertions: &mut Vec<(usize, String)>,
) {
    if lone_param(params).is_none_or(|p| p.typed) {
        return;
    }
    insertions.push((
        insert_at,
        format!("/** @type {{{}}} */ ", types.jsdoc_type()),
    ));
}

/// Byte offset of an arrow function's `=>` token — where its return
/// annotation has to go, since anything later would land inside the
/// body.
///
/// Only the span between the parameter list and the body is scanned,
/// and comments are skipped so that a `// x => y` note between the two
/// can't be mistaken for the token. Callers only reach this when the
/// arrow has no return type of its own, so nothing but whitespace,
/// comments and the token itself can appear in that span.
fn arrow_token_pos(source: &str, params_end: usize, body_start: usize) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut i = params_end;
    while i + 1 < body_start.min(bytes.len()) {
        match (bytes[i], bytes[i + 1]) {
            (b'/', b'/') => {
                i += 2;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
            }
            (b'/', b'*') => {
                i += 2;
                while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                    i += 1;
                }
                i += 2;
            }
            (b'=', b'>') => return Some(i),
            _ => i += 1,
        }
    }
    None
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
    let Some(param) = lone_param(&arrow.params).filter(|p| !p.typed) else {
        return;
    };
    insertions.push((param.end, format!(": {event_type}")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Default settings plus the pre-SvelteKit-3 hook-types module.
    /// Tests that care about the SvelteKit 3 module say so explicitly.
    fn opts() -> KitInjectOptions<'static> {
        static SETTINGS: std::sync::LazyLock<KitFilesSettings> =
            std::sync::LazyLock::new(KitFilesSettings::default);
        KitInjectOptions {
            settings: &SETTINGS,
            hooks_types_module: "@sveltejs/kit",
        }
    }

    fn inject(path: &Path, source: &str) -> Option<String> {
        super::inject(path, source, &opts()).map(|i| i.text)
    }

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
    fn injects_on_arrow_server_handler() {
        // `export const GET = (event) => …` is a handler too; the return
        // annotation lands on the `=>` token.
        let source = "export const GET = (event) /* a => b */ => new Response('x');";
        let got = inject(&server_path(), source).unwrap();
        assert!(
            got.contains(
                "(event: import('./$types.js').RequestEvent) /* a => b */ : Response | Promise<Response> => new Response('x')"
            ),
            "got: {got}"
        );
        let source = "export const POST = async (event) => new Response('x');";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains(": Promise<Response> =>"), "got: {got}");
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
    fn server_endpoint_types_page_options_and_entries() {
        // Upstream annotates page options and `entries` on every kit
        // route file, `+server` included.
        let source = "export const ssr = true;\nexport const entries = () => [];\n";
        let got = inject(&server_path(), source).unwrap();
        assert!(got.contains("ssr: boolean = true"), "{got}");
        // The annotation lands where upstream inserts it: at the `=>`.
        assert!(
            got.contains("entries = () : ReturnType<import('./$types.js').EntryGenerator> => []"),
            "{got}"
        );
    }

    #[test]
    fn actions_get_satisfies_trailer_and_entries_a_return_type() {
        let source = "export const actions = { default: async (e) => {} };\nexport function entries() { return []; }\n";
        let got = inject(&PathBuf::from("src/routes/+page.server.ts"), source).unwrap();
        assert!(got.contains("} satisfies import('./$types.js').Actions;"));
        // Inserted at the body's `{`, as upstream does.
        assert!(got.contains(
            "export function entries() : ReturnType<import('./$types.js').EntryGenerator> {"
        ));
        // A layout has no `entries`; an annotated `actions` is left alone.
        let layout = inject(&layout_path(), "export function entries() { return []; }\n");
        assert!(layout.is_none());
        let typed = inject(
            &PathBuf::from("src/routes/+page.server.ts"),
            "export const actions: Actions = {};\n",
        );
        assert!(typed.is_none());
    }

    #[test]
    fn js_actions_and_entries_get_jsdoc_forms() {
        let source = "export const actions = { default: async (e) => {} };\nexport function entries() { return []; }\n";
        let got = inject(&PathBuf::from("src/routes/+page.server.js"), source).unwrap();
        assert!(got.contains(
            "export const actions = /** @satisfies {import('./$types.js').Actions} */ ({ default: async (e) => {} });"
        ));
        assert!(got.contains(
            "/** @type {import('./$types.js').EntryGenerator} */ export function entries()"
        ));
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
    fn js_jsdoc_typing_follows_typescript_attachment() {
        // Typed: the last JSDoc block before the export (a line comment
        // in between doesn't detach it), a same-line JSDoc on the
        // declaration or on the function value, and a `@param` naming
        // the parameter.
        for source in [
            "/** @type {import('./$types').PageLoad} */\n// note\nexport function load(e) { return {}; }",
            "export const /** @type {import('./$types').PageLoad} */ load = (e) => ({});",
            "export const load = /** @type {import('./$types').PageLoad} */ (e) => ({});",
            "/** @param {import('./$types').PageLoadEvent} e */\nexport const load = (e) => ({});",
            "/** @param {import('./$types').PageLoadEvent} */\nexport const load = ({ url }) => ({});",
        ] {
            assert!(
                inject(&page_js_path(), source).is_none(),
                "should not double-type: {source}"
            );
        }
        // Untyped: `@typedef` is not `@type`, an earlier JSDoc block is
        // shadowed by a later one, a same-line block before `export`
        // is trailing trivia of the previous statement, and a `@param`
        // for another name doesn't cover the parameter.
        for source in [
            "/** @typedef {{a:1}} T */ export const load = (e) => ({});",
            "/** @type {import('./$types').PageLoad} */\n/** Loads. */\nexport function load(e) { return {}; }",
            "const a = 1; /** @type {import('./$types').PageLoad} */\nexport function load(e) { return { a }; }",
            "/** @param {string} other */\nexport function load(e) { return {}; }",
        ] {
            let got = inject(&page_js_path(), source)
                .unwrap_or_else(|| panic!("untyped load must inject: {source}"));
            assert!(got.contains("PageLoadEvent} "), "{got}");
        }
    }

    #[test]
    fn export_default_function_is_an_export() {
        // `export default function GET` — the first modifier is still
        // `export`, so upstream registers it under its own name.
        let source = "export default function GET(e) { return new Response(e.url); }";
        let got = inject(&server_path(), source).unwrap();
        assert!(
            got.contains(
                "GET(e: import('./$types.js').RequestEvent) : Response | Promise<Response> {"
            ),
            "{got}"
        );
    }

    #[test]
    fn parameter_annotation_goes_after_the_default_value() {
        // Upstream splices at the end of the whole parameter, default
        // included — the result doesn't parse, and upstream reports the
        // syntax errors that follow.
        let source = "export function load({ url } = {}) { return {}; }";
        let got = inject(&page_path(), source).unwrap();
        assert!(
            got.contains("load({ url } = {}: import('./$types.js').PageLoadEvent)"),
            "{got}"
        );
        let source = "export const load = (...args) => ({});";
        let got = inject(&page_path(), source).unwrap();
        assert!(
            got.contains("(...args: import('./$types.js').PageLoadEvent)"),
            "{got}"
        );
        let got = inject(&page_js_path(), source).unwrap();
        assert!(
            got.contains("/** @param {import('./$types.js').PageLoadEvent} args */"),
            "{got}"
        );
    }

    #[test]
    fn js_jsdoc_typing_must_sit_directly_on_the_export() {
        // An earlier `@type` block belongs to `a`, not to `load`; the
        // plain comment in between is not JSDoc. `load` is untyped.
        let source = "/** @type {string} */\nconst a = 'x';\n/* plain note */\nexport function load(event) { return { u: event.url.pathname, a }; }";
        let got = inject(&page_js_path(), source).expect("untyped load must inject");
        assert!(got.contains("PageLoadEvent"), "{got}");
    }

    // ===== Hooks and param matchers =====================================
    //
    // The expected strings below are not invented: they were taken from
    // what upstream svelte-check --tsgo actually wrote to
    // `.svelte-kit/.svelte-check/svelte/src/…` for these exact inputs.
    // The spacing is upstream's too — the return annotation carries a
    // leading space (it lands before the `=>` or `{`, which the source
    // already separated with one) and a trailing one.

    fn hooks_server_path() -> PathBuf {
        PathBuf::from("src/hooks.server.ts")
    }
    fn hooks_client_path() -> PathBuf {
        PathBuf::from("src/hooks.client.ts")
    }
    fn hooks_universal_path() -> PathBuf {
        PathBuf::from("src/hooks.ts")
    }
    fn params_path() -> PathBuf {
        PathBuf::from("src/params/fruit.ts")
    }

    #[test]
    fn server_hooks_arrow_gets_param_and_return() {
        let source =
            "export const handle = async ({ event, resolve }) => {\n\treturn resolve(event);\n};\n";
        let got = inject(&hooks_server_path(), source).unwrap();
        assert_eq!(
            got,
            "export const handle = async ({ event, resolve }: Parameters<import('@sveltejs/kit').Handle>[0]) : ReturnType<import('@sveltejs/kit').Handle> => {\n\treturn resolve(event);\n};\n"
        );
    }

    #[test]
    fn server_hooks_function_declaration_gets_param_and_return() {
        let source =
            "export function handleFetch({ request, fetch }) {\n\treturn fetch(request);\n}\n";
        let got = inject(&hooks_server_path(), source).unwrap();
        assert_eq!(
            got,
            "export function handleFetch({ request, fetch }: Parameters<import('@sveltejs/kit').HandleFetch>[0]) : ReturnType<import('@sveltejs/kit').HandleFetch> {\n\treturn fetch(request);\n}\n"
        );
    }

    #[test]
    fn hooks_types_module_follows_the_installed_kit_major() {
        let source = "export const reroute = ({ url }) => url.pathname;\n";
        let settings = KitFilesSettings::default();
        let got = super::inject(
            &hooks_universal_path(),
            source,
            &KitInjectOptions {
                settings: &settings,
                hooks_types_module: "@sveltejs/kit/hooks",
            },
        )
        .unwrap()
        .text;
        assert_eq!(
            got,
            "export const reroute = ({ url }: Parameters<import('@sveltejs/kit/hooks').Reroute>[0]) : ReturnType<import('@sveltejs/kit/hooks').Reroute> => url.pathname;\n"
        );
    }

    #[test]
    fn client_hooks_type_error_differs_from_server() {
        let source = "export const handleError = ({ error, event }) => ({ message: 'oops' });\n";
        let got = inject(&hooks_client_path(), source).unwrap();
        assert!(
            got.contains("HandleClientError") && !got.contains("HandleServerError"),
            "client hooks must project HandleClientError: {got}"
        );
    }

    #[test]
    fn param_matcher_gets_concrete_string_to_boolean() {
        let source = "export function match(param) {\n\treturn param === 'apple';\n}\n";
        let got = inject(&params_path(), source).unwrap();
        assert_eq!(
            got,
            "export function match(param: string) : boolean {\n\treturn param === 'apple';\n}\n"
        );
    }

    #[test]
    fn hook_exports_outside_the_known_set_are_left_alone() {
        // `handle` belongs to server hooks, not client hooks; `match`
        // belongs to params, not hooks. An export the role doesn't
        // recognise is user code and stays untouched.
        assert!(
            inject(
                &hooks_client_path(),
                "export const handle = (input) => input;\n"
            )
            .is_none()
        );
        assert!(
            inject(
                &hooks_server_path(),
                "export function match(p) { return !!p; }\n"
            )
            .is_none()
        );
        assert!(inject(&hooks_server_path(), "export const helper = (x) => x;\n").is_none());
    }

    #[test]
    fn user_typed_hooks_are_left_alone() {
        // An annotated variable is upstream's `hasTypeDefinition`; so is
        // a `satisfies` wrapper. Either way the user owns the signature.
        for source in [
            "export const handle: import('@sveltejs/kit').Handle = ({ event, resolve }) => resolve(event);\n",
            "export const handle = (({ event, resolve }) => resolve(event)) satisfies import('@sveltejs/kit').Handle;\n",
            "export const handle = async ({ event, resolve }: any) => resolve(event);\n",
        ] {
            assert!(
                inject(&hooks_server_path(), source).is_none(),
                "should not re-type: {source}"
            );
        }
    }

    #[test]
    fn multi_parameter_hooks_are_left_alone() {
        // Upstream's gate is `parameters.length === 1`. A hook written
        // with a second parameter isn't the shape the type describes,
        // and annotating it would report the wrong error.
        let source = "export const handle = ({ event }, extra) => extra;\n";
        assert!(inject(&hooks_server_path(), source).is_none());
    }

    #[test]
    fn arrow_token_is_found_past_a_comment_containing_one() {
        // The `=>` inside the comment must not be mistaken for the
        // token, or the annotation lands mid-comment and corrupts it.
        let source = "export const reroute = ({ url }) /* a => b */ => url.pathname;\n";
        let got = inject(&hooks_universal_path(), source).unwrap();
        assert_eq!(
            got,
            "export const reroute = ({ url }: Parameters<import('@sveltejs/kit').Reroute>[0]) /* a => b */ : ReturnType<import('@sveltejs/kit').Reroute> => url.pathname;\n"
        );
    }

    #[test]
    fn js_hooks_get_a_jsdoc_type_instead() {
        let source = "export const handle = async ({ event, resolve }) => resolve(event);\n";
        let got = inject(&PathBuf::from("src/hooks.server.js"), source).unwrap();
        assert_eq!(
            got,
            "export const handle = /** @type {import('@sveltejs/kit').Handle} */ async ({ event, resolve }) => resolve(event);\n"
        );
    }

    #[test]
    fn js_param_matcher_gets_a_function_type_jsdoc() {
        let source = "export function match(param) {\n\treturn !!param;\n}\n";
        let got = inject(&PathBuf::from("src/params/fruit.js"), source).unwrap();
        assert_eq!(
            got,
            "/** @type {(arg0: string) => boolean} */ export function match(param) {\n\treturn !!param;\n}\n"
        );
    }

    #[test]
    fn hooks_injections_add_no_lines() {
        let source =
            "export const handle = async ({ event, resolve }) => {\n\treturn resolve(event);\n};\n";
        let got = inject(&hooks_server_path(), source).unwrap();
        assert_eq!(source.matches('\n').count(), got.matches('\n').count());
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
