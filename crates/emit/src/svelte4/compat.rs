//! Svelte-4 compatibility helpers.
//!
//! Two concerns live here:
//!   1. Detection — `is_svelte4_component`, the various
//!      `has_*` / `contains_*` / `is_runes_mode` predicates that decide
//!      whether to apply the rewrites or widening intersections.
//!   2. Source-text rewrites of the script body (exported-prop type
//!      assertions) and the `$$slots` / `$$props` / `$$restProps`
//!      ambient emission.
//!
//! When Svelte 4 is officially retired this whole module gets deleted
//! along with the `// SVELTE-4-COMPAT` callsites in `lib.rs`. See
//! `design/phase_g/DESIGN.md`.

use std::ops::Range;

use smol_str::SmolStr;
use svn_parser::ScriptLang;

use crate::sveltekit;

/// A top-level `let` declarator in the instance script body. Byte
/// positions are absolute offsets into the emit buffer.
struct LetDeclarator {
    name: SmolStr,
    /// Position of the binding name.
    name_start: usize,
    /// Position right after the binding name, where `!` or `: T` goes.
    name_end: usize,
    has_type_annotation: bool,
    /// A `/** @type {…} */` JSDoc block leads the statement.
    has_jsdoc_type: bool,
    init: DeclaratorInit,
    /// Position right after the statement's last declarator (before
    /// its `;`, if any).
    list_end: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DeclaratorInit {
    Absent,
    /// `= undefined` or `= null`.
    Nullish,
    /// `= true` or `= false`.
    BoolLiteral,
    Other,
}

/// Parse the script body spliced at `body` inside `out` and list its
/// top-level `let` declarators that bind a plain identifier (`const`
/// ones with `kind = Const`).
///
/// The in-place rewrites need to know where a declaration's name, type
/// annotation and statement end sit. A byte scan misreads text that
/// only looks like code — the word `let` in a comment followed by an
/// unclosed `(`, an initializer continued onto the next line by a
/// trailing `+` — so the positions come from the parser. Only
/// top-level statements count: every rewrite target is a
/// component-scope binding, and a nested `let` with the same name is a
/// different variable.
fn collect_top_level_lets(out: &str, body: &Range<usize>) -> Vec<LetDeclarator> {
    collect_top_level_declarators(out, body, oxc_ast::ast::VariableDeclarationKind::Let)
}

fn collect_top_level_declarators(
    out: &str,
    body: &Range<usize>,
    kind: oxc_ast::ast::VariableDeclarationKind,
) -> Vec<LetDeclarator> {
    use oxc_ast::ast::{BindingPattern, Declaration, Expression, Statement};

    let Some(src) = out.get(body.clone()) else {
        return Vec::new();
    };
    let alloc = oxc_allocator::Allocator::default();
    // Always parse as TypeScript. It accepts every JS body, and a JS
    // overlay's body can still hold TS-only syntax (a `lang`-less script
    // with a type annotation) that would make a JS parse give up.
    let parsed = svn_parser::parse_script_body(&alloc, src, ScriptLang::Ts);
    if parsed.panicked {
        return Vec::new();
    }
    let mut decls = Vec::new();
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::VariableDeclaration(d) => d,
            Statement::ExportDeclaration(e) => match &e.declaration {
                Declaration::VariableDeclaration(d) => d,
                _ => continue,
            },
            // None of these can declare a `let`.
            Statement::FunctionDeclaration(_)
            | Statement::ClassDeclaration(_)
            | Statement::ImportDeclaration(_)
            | Statement::ExportNamedDeclaration(_)
            | Statement::ExportFromDeclaration(_)
            | Statement::ExportAllDeclaration(_)
            | Statement::ExportDefaultDeclaration(_)
            | Statement::TSInterfaceDeclaration(_)
            | Statement::TSTypeAliasDeclaration(_)
            | Statement::TSEnumDeclaration(_)
            | Statement::TSExternalModuleDeclaration(_)
            | Statement::TSNamespaceDeclaration(_)
            | Statement::TSGlobalDeclaration(_)
            | Statement::TSImportEqualsDeclaration(_)
            | Statement::TSExportAssignment(_)
            | Statement::TSNamespaceExportDeclaration(_) => continue,
            svn_analyze::non_declaration_statement!() => continue,
        };
        if decl.kind != kind {
            continue;
        }
        let Some(list_end) = decl
            .declarations
            .last()
            .map(|d| body.start + d.span.end as usize)
        else {
            continue;
        };
        // `ts.getJSDocType(declaration)`: a JSDoc block right before
        // the statement that carries an `@type` tag.
        let has_jsdoc_type = parsed.program.comments.iter().any(|c| {
            c.is_jsdoc()
                && c.is_leading()
                && c.attached_to == decl.span.start
                && src
                    .get(c.content_span().start as usize..c.content_span().end as usize)
                    .is_some_and(|text| text.contains("@type"))
        });
        for d in &decl.declarations {
            let BindingPattern::BindingIdentifier(id) = &d.id else {
                continue;
            };
            let init = match &d.init {
                None => DeclaratorInit::Absent,
                Some(Expression::NullLiteral(_)) => DeclaratorInit::Nullish,
                Some(Expression::Identifier(i)) if i.name.as_str() == "undefined" => {
                    DeclaratorInit::Nullish
                }
                Some(Expression::BooleanLiteral(_)) => DeclaratorInit::BoolLiteral,
                Some(_) => DeclaratorInit::Other,
            };
            decls.push(LetDeclarator {
                name: SmolStr::from(id.name.as_str()),
                name_start: body.start + id.span.start as usize,
                name_end: body.start + id.span.end as usize,
                has_type_annotation: d.type_annotation.is_some(),
                has_jsdoc_type,
                init,
                list_end,
            });
        }
    }
    decls
}

fn is_target(target_names: &[SmolStr], name: &str) -> bool {
    target_names.iter().any(|t| t == name)
}

/// Splice `(position, text)` insertions into `out` in one rebuild.
/// Positions must be ascending. Returns the edits as `(position,
/// length)` pairs in pre-rewrite coordinates, the shape
/// `EmitBuffer::adjust_token_map_for_insertions` re-anchors with.
pub(crate) fn splice_insertions(
    out: &mut String,
    insertions: &[(usize, String)],
) -> Vec<(u32, u32)> {
    if insertions.is_empty() {
        return Vec::new();
    }
    let original = std::mem::take(out);
    let extra: usize = insertions.iter().map(|(_, text)| text.len()).sum();
    let mut rebuilt = String::with_capacity(original.len() + extra);
    let mut edits = Vec::with_capacity(insertions.len());
    let mut cursor = 0;
    for (pos, text) in insertions {
        rebuilt.push_str(&original[cursor..*pos]);
        rebuilt.push_str(text);
        edits.push((*pos as u32, text.len() as u32));
        cursor = *pos;
    }
    rebuilt.push_str(&original[cursor..]);
    *out = rebuilt;
    edits
}

/// Does the parsed template fragment contain a `<slot>` element?
pub(crate) fn fragment_contains_slot(fragment: &svn_parser::Fragment) -> bool {
    fragment_has_slot_where(fragment, &|_| true)
}

/// Does the fragment contain a default slot? Upstream's
/// `__sveltets_2_PropsWithChildren` widens the props with `children`
/// only when the slots type has a `default` key, and svelte2tsx keys a
/// `<slot>` by the raw text of the first value chunk of its first
/// attribute called `name` — `default` when there is none, `undefined`
/// when that chunk is a `{…}` expression (`slot.ts` `handleSlot`).
pub(crate) fn fragment_contains_default_slot(
    fragment: &svn_parser::Fragment,
    source: &str,
) -> bool {
    use svn_parser::Attribute;
    fragment_has_slot_where(fragment, &|slot| {
        let name_attr = slot.attributes.iter().find(|a| match a {
            Attribute::Plain(p) => p.name.as_str() == "name",
            Attribute::Expression(x) => x.name.as_str() == "name",
            Attribute::Shorthand(x) => x.name.as_str() == "name",
            Attribute::Directive(d) => d.name.as_str() == "name",
            Attribute::Spread(_) | Attribute::Comment(_) => false,
        });
        match name_attr {
            None => true,
            Some(Attribute::Plain(p)) => matches!(
                p.value.as_ref().and_then(|v| v.parts.first()),
                Some(svn_parser::AttrValuePart::Text { range })
                    if source.get(range.start as usize..range.end as usize) == Some("default")
            ),
            Some(_) => false,
        }
    })
}

/// Walk every element of the fragment (through blocks and component
/// children) and report whether any `<slot>` satisfies `pred`. Tag
/// names are exact: `<slotfoo>` / `<Slot>` do not match, and comments
/// and text are never elements.
fn fragment_has_slot_where(
    fragment: &svn_parser::Fragment,
    pred: &dyn Fn(&svn_parser::Element) -> bool,
) -> bool {
    fragment_has_element_where(fragment, &|e| e.name.as_str() == "slot" && pred(e))
}

/// Walk every element of the fragment (through blocks and component
/// children) and report whether any satisfies `pred`. Comments and
/// text are never elements.
fn fragment_has_element_where(
    fragment: &svn_parser::Fragment,
    pred: &dyn Fn(&svn_parser::Element) -> bool,
) -> bool {
    use svn_parser::Node;
    for node in &fragment.nodes {
        let hit = match node {
            Node::Element(e) => pred(e) || fragment_has_element_where(&e.children, pred),
            Node::Component(c) => fragment_has_element_where(&c.children, pred),
            Node::SvelteElement(e) => fragment_has_element_where(&e.children, pred),
            Node::IfBlock(b) => {
                fragment_has_element_where(&b.consequent, pred)
                    || b.elseif_arms
                        .iter()
                        .any(|arm| fragment_has_element_where(&arm.body, pred))
                    || b.alternate
                        .as_ref()
                        .is_some_and(|alt| fragment_has_element_where(alt, pred))
            }
            Node::EachBlock(b) => {
                fragment_has_element_where(&b.body, pred)
                    || b.alternate
                        .as_ref()
                        .is_some_and(|alt| fragment_has_element_where(alt, pred))
            }
            Node::AwaitBlock(b) => {
                b.pending
                    .as_ref()
                    .is_some_and(|p| fragment_has_element_where(p, pred))
                    || b.then_branch
                        .as_ref()
                        .is_some_and(|t| fragment_has_element_where(&t.body, pred))
                    || b.catch_branch
                        .as_ref()
                        .is_some_and(|c| fragment_has_element_where(&c.body, pred))
            }
            Node::KeyBlock(b) => fragment_has_element_where(&b.body, pred),
            Node::SnippetBlock(b) => fragment_has_element_where(&b.body, pred),
            Node::Text(_) | Node::Comment(_) | Node::Interpolation(_) => false,
        };
        if hit {
            return true;
        }
    }
    false
}

/// SVELTE-4-COMPAT — v0.3 Item 3. Detect whether the component has a
/// `$$Events` interface or type declaration in its instance or module
/// script via an AST walk over the parsed scripts. When true, the
/// default-export declaration intersects with
/// `& { readonly __svn_events: $$Events }` so consumers resolve to
/// `__svn_ensure_component`'s typed overload and get narrowed
/// `$on("evt", handler)` signatures.
///
/// Reviewer follow-up #3a: pre-fix this scanned the raw source for
/// the substrings `"interface $$Events"` / `"type $$Events "`
/// (with trailing space), which (a) false-fired on comments and
/// string literals containing those phrases, (b) silently missed
/// `type $$Events=…` (no whitespace before `=`) and `type
/// $$Events<T>=…` (generic), and (c) missed
/// `export interface $$Events { … }` in some forms. AST-based
/// detection covers every well-formed declaration shape and stays
/// blind to comments / strings / unrelated identifiers.
pub(crate) fn has_strict_events_ast(
    parsed_instance: Option<&svn_parser::ParsedScript<'_>>,
    _parsed_module: Option<&svn_parser::ParsedScript<'_>>,
) -> bool {
    // Reviewer follow-up #5 (round 4): pre-fix this also scanned
    // the module script. Upstream svelte2tsx
    // (`processModuleScriptTag.ts:119`) explicitly REJECTS a
    // `$$Events` declaration in `<script module>` — only the
    // instance script's declaration is the strict-event opt-in.
    // Recognizing the module-script form would silently widen
    // the event surface based on a declaration upstream wouldn't
    // allow.
    //
    // We don't currently surface a diagnostic for the
    // module-script case (lint-level concern, deferred). Ignore
    // the module-script declaration so the rest of the pipeline
    // behaves as if `$$Events` is absent.
    let scan = |program: &oxc_ast::ast::Program<'_>| -> bool {
        program.body.iter().any(statement_declares_events)
    };
    parsed_instance.is_some_and(|p| scan(&p.program))
}

/// Whether the instance script's `$$Events` declaration names any
/// event, as upstream's `ComponentEventsFromInterface.extractEvents`
/// counts them: the property signatures of an interface body, of a
/// type-literal alias, or of the type-literal members of an
/// intersection alias. Anything else (`type $$Events = Base`, an empty
/// interface, method signatures) contributes no events. With several
/// declarations the last one counts.
pub(crate) fn strict_events_decl_has_events(
    parsed_instance: Option<&svn_parser::ParsedScript<'_>>,
) -> bool {
    use oxc_ast::ast::{Declaration, Statement, TSSignature, TSType};
    fn has_properties(members: &[TSSignature<'_>]) -> bool {
        members
            .iter()
            .any(|m| matches!(m, TSSignature::TSPropertySignature(_)))
    }
    fn alias_has_events(ty: &TSType<'_>) -> bool {
        match ty {
            TSType::TSTypeLiteral(lit) => has_properties(&lit.members),
            TSType::TSIntersectionType(i) => i.types.iter().any(|t| match t {
                TSType::TSTypeLiteral(lit) => has_properties(&lit.members),
                _ => false,
            }),
            _ => false,
        }
    }
    let Some(parsed) = parsed_instance else {
        return false;
    };
    let mut last: Option<bool> = None;
    for stmt in &parsed.program.body {
        let decl = match stmt {
            Statement::TSInterfaceDeclaration(d) if d.id.name == "$$Events" => {
                Some(has_properties(&d.body.body))
            }
            Statement::TSTypeAliasDeclaration(d) if d.id.name == "$$Events" => {
                Some(alias_has_events(&d.type_annotation))
            }
            Statement::ExportDeclaration(e) => match &e.declaration {
                Declaration::TSInterfaceDeclaration(d) if d.id.name == "$$Events" => {
                    Some(has_properties(&d.body.body))
                }
                Declaration::TSTypeAliasDeclaration(d) if d.id.name == "$$Events" => {
                    Some(alias_has_events(&d.type_annotation))
                }
                _ => None,
            },
            _ => None,
        };
        if decl.is_some() {
            last = decl;
        }
    }
    last.unwrap_or(false)
}

/// True when `stmt` is an `interface $$Events` or `type $$Events`
/// declaration (including the `export interface` / `export type`
/// re-exporting forms). Used by [`has_strict_events_ast`].
fn statement_declares_events(stmt: &oxc_ast::ast::Statement<'_>) -> bool {
    statement_declares_named_type(stmt, "$$Events")
}

/// SlotHandler PLAN Stage 5: detect an `interface $$Slots` /
/// `type $$Slots` declaration in the instance script via AST. When
/// present, the render-fn return's `slots:` field uses `$$Slots`
/// instead of the synthesised slot-defs (mirrors upstream's
/// `uses$$SlotsInterface` flag in `createRenderFunction.ts:125`).
///
/// Module-script declarations don't count — upstream's
/// `processModuleScriptTag.ts:127` rejects `$$Slots` there. Same
/// rule we apply for `$$Events`.
pub(crate) fn has_strict_slots_ast(parsed_instance: Option<&svn_parser::ParsedScript<'_>>) -> bool {
    parsed_instance.is_some_and(|p| {
        p.program
            .body
            .iter()
            .any(|s| statement_declares_named_type(s, "$$Slots"))
    })
}

/// Generic helper: true when `stmt` is an `interface NAME` or
/// `type NAME` declaration (with or without `export`).
///
/// Top-level-only contract: only the two type-declaration shapes (and
/// their `export`-wrapped forms) can satisfy the check — every other
/// declaration kind is enumerated below so the match stays exhaustive
/// and a new oxc `Statement` variant fails compilation instead of
/// silently reading as "not a type declaration".
fn statement_declares_named_type(stmt: &oxc_ast::ast::Statement<'_>, name: &str) -> bool {
    use oxc_ast::ast::{Declaration, Statement};
    match stmt {
        Statement::TSInterfaceDeclaration(d) => d.id.name.as_str() == name,
        Statement::TSTypeAliasDeclaration(d) => d.id.name.as_str() == name,
        Statement::ExportDeclaration(e) => match &e.declaration {
            Declaration::TSInterfaceDeclaration(d) => d.id.name.as_str() == name,
            Declaration::TSTypeAliasDeclaration(d) => d.id.name.as_str() == name,
            _ => false,
        },
        Statement::ExportNamedDeclaration(_) | Statement::ExportFromDeclaration(_) => false,
        Statement::VariableDeclaration(_)
        | Statement::FunctionDeclaration(_)
        | Statement::ClassDeclaration(_)
        | Statement::ImportDeclaration(_)
        | Statement::ExportAllDeclaration(_)
        | Statement::ExportDefaultDeclaration(_)
        | Statement::TSEnumDeclaration(_)
        | Statement::TSExternalModuleDeclaration(_)
        | Statement::TSNamespaceDeclaration(_)
        | Statement::TSGlobalDeclaration(_)
        | Statement::TSImportEqualsDeclaration(_)
        | Statement::TSExportAssignment(_)
        | Statement::TSNamespaceExportDeclaration(_) => false,
        svn_analyze::non_declaration_statement!() => false,
    }
}

/// SVELTE-4-COMPAT: detect the `strictEvents` opt-in, which turns on
/// event-typing narrowing without a `$$Events` interface. Upstream
/// (`htmlxtojsx_v2/index.ts`) enables it when any `<script>` or
/// `<style>` tag of the file — the component's own sections or one
/// written inside the markup — carries an attribute named exactly
/// `strictEvents`, whatever its value.
pub(crate) fn has_strict_events_attr(
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
) -> bool {
    const NAME: &str = "strictEvents";
    let on_section = |attrs: &[svn_parser::ScriptAttr]| attrs.iter().any(|a| a.name == NAME);
    doc.instance_script
        .as_ref()
        .is_some_and(|s| on_section(&s.attrs))
        || doc
            .module_script
            .as_ref()
            .is_some_and(|s| on_section(&s.attrs))
        || doc.style.as_ref().is_some_and(|s| on_section(&s.attrs))
        || fragment_has_element_where(fragment, &|e| {
            matches!(e.name.as_str(), "script" | "style")
                && e.attributes.iter().any(
                    |a| matches!(a, svn_parser::Attribute::Plain(p) if p.name.as_str() == NAME),
                )
        })
}

/// Infer Svelte 5 runes mode the way upstream svelte2tsx does
/// (`ExportedNames.isRunesMode`): the script or a template expression
/// references one of the `$state` / `$derived` / `$effect` globals,
/// something is declared from `$props()`, an `await` sits outside any
/// function, or the document sets `<svelte:options runes>`. Nothing
/// else counts — a rune name in markup text or a comment is not a
/// reference, and `$inspect` alone does not switch modes.
///
/// A `$state` that resolves to a store subscription (`const state =
/// writable(…)` in a Svelte-4 component) is not a global either, so
/// the top-level bindings of both scripts are excluded first.
pub(crate) fn is_runes_mode(
    doc: &svn_parser::Document<'_>,
    fragment: &svn_parser::Fragment,
    parsed_instance: Option<&svn_parser::ParsedScript<'_>>,
    parsed_module: Option<&svn_parser::ParsedScript<'_>>,
) -> bool {
    let source = doc.source;
    // An explicit `<svelte:options runes>` / `runes={true}` forces runes
    // ON. `runes={false}` cannot force runes OFF here (upstream OR-s the
    // option with the script signals), so only `Some(true)` participates.
    if svn_parser::runes_option(fragment, source) == Some(true) {
        return true;
    }
    let mut bound: std::collections::HashSet<String> = std::collections::HashSet::new();
    for parsed in [parsed_instance, parsed_module].into_iter().flatten() {
        svn_analyze::collect_top_level_bindings(&parsed.program, &mut bound);
    }
    let mut probe = svn_analyze::RunesProbe::new(svn_analyze::RunesRule::Svelte2tsx, &bound);
    for parsed in [parsed_instance, parsed_module].into_iter().flatten() {
        probe.scan_program(&parsed.program);
        if probe.found {
            return true;
        }
    }
    // Template expressions: `{let x = $state(0)}`, `{await p}`. Only
    // worth parsing when the text carries a marker at all.
    let has_marker = doc.template.text_runs.iter().any(|run| {
        source
            .get(run.start as usize..run.end as usize)
            .is_some_and(|t| {
                ["$state", "$derived", "$effect", "await"]
                    .iter()
                    .any(|m| t.contains(m))
            })
    });
    if !has_marker {
        return false;
    }
    let alloc = oxc_allocator::Allocator::default();
    for expr in svn_analyze::template_expression_ranges(fragment) {
        let Some(text) = source.get(expr.range.start as usize..expr.range.end as usize) else {
            continue;
        };
        let wrapped = if expr.is_declaration {
            format!("let {text}\n;")
        } else {
            format!("({text}\n);")
        };
        let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
        probe.scan_program(&parsed.program);
        if probe.found {
            return true;
        }
    }
    false
}

/// SVELTE-4-COMPAT: emit `let $$slots = …; let $$props = …; let
/// $$restProps = …;` at the top of the render function for each
/// ambient the component refers to (an identifier reference in a
/// script or template expression — see `svn_analyze::find_ambient_refs`).
///
/// Types: `Record<string, any>` for all three. Upstream's
/// `__sveltets_2_slotsType({…slot names…})` is more precise (each
/// slot is typed as `boolean | ''`), but that requires walking the
/// template to collect slot names and emit a shape literal.
pub(crate) fn emit_svelte4_ambients(out: &mut String, refs: svn_analyze::AmbientRefs, is_ts: bool) {
    // In TS overlays we emit inline `: T` annotations. In JS overlays
    // we must not — tsgo fires TS8010 and aborts project-wide once
    // hit, silently suppressing every legitimate diagnostic
    // elsewhere. Emit JSDoc casts on the RHS for JS overlays.
    if refs.slots {
        if is_ts {
            out.push_str("    let $$slots: Record<string, boolean | undefined> = {};\n");
        } else {
            out.push_str(
                "    let $$slots = /** @type {Record<string, boolean | undefined>} */ ({});\n",
            );
        }
        out.push_str("    void $$slots;\n");
    }
    if refs.rest_props {
        if is_ts {
            out.push_str("    let $$restProps: Record<string, any> = {};\n");
        } else {
            out.push_str("    let $$restProps = /** @type {Record<string, any>} */ ({});\n");
        }
        out.push_str("    void $$restProps;\n");
    }
    if refs.props {
        if is_ts {
            out.push_str("    let $$props: Record<string, any> = {};\n");
        } else {
            out.push_str("    let $$props = /** @type {Record<string, any>} */ ({});\n");
        }
        out.push_str("    void $$props;\n");
    }
}
/// The SvelteKit type of a route-file prop upstream names without a
/// declared type: `data`, `form` and `snapshot` in `+page` / `+layout`
/// components (`ExportedNames.ts`, `kitType`).
fn kit_prop_type(name: &str, route_kind: Option<sveltekit::RouteKind>) -> Option<String> {
    if matches!(route_kind, None | Some(sveltekit::RouteKind::Error)) {
        return None;
    }
    let ty = match name {
        "data" => match route_kind? {
            sveltekit::RouteKind::Layout => "LayoutData",
            sveltekit::RouteKind::Page => "PageData",
            // `+error.svelte` is not one of upstream's `kitPageFiles`.
            sveltekit::RouteKind::Error => return None,
        },
        "form" => "ActionData",
        "snapshot" => "Snapshot",
        _ => return None,
    };
    Some(format!("import('./$types.js').{ty}"))
}

/// Append `;NAME = __svn_any(NAME);` (in ignore comments) after each
/// exported `let` whose declared type TypeScript would otherwise
/// narrow away — upstream `ExportedNames.propTypeAssertToUserDefined`.
///
/// Three declaration shapes qualify, and the one reassignment covers
/// all three: no initializer (the assignment makes the prop count as
/// initialised, and an untyped one becomes `any`); a declared type —
/// TS annotation or JSDoc `@type` — with an initializer (the
/// assignment resets the literal narrowing back to the annotation);
/// and an untyped `= true` / `= false` (TypeScript keeps the literal
/// type for the read that builds the props object, so a consumer
/// passing the other value would be an error). A SvelteKit route
/// file's `data` / `form` / `snapshot` without a declared type also
/// gets its `$types` annotation, as a TS annotation in ignore
/// comments or a JSDoc `@type` in a JS overlay.
pub(crate) fn assert_exported_prop_types_in_place(
    out: &mut String,
    body: &Range<usize>,
    exported_names: &[SmolStr],
    route_kind: Option<sveltekit::RouteKind>,
    is_ts: bool,
) -> Vec<(u32, u32)> {
    if exported_names.is_empty() {
        return Vec::new();
    }
    let mut insertions: Vec<(usize, String)> = Vec::new();
    for d in collect_top_level_lets(out, body) {
        if !is_target(exported_names, &d.name) {
            continue;
        }
        let has_type = d.has_type_annotation || d.has_jsdoc_type;
        let kit_type = if has_type {
            None
        } else {
            kit_prop_type(&d.name, route_kind)
        };
        let widen =
            d.init == DeclaratorInit::Absent || has_type || d.init == DeclaratorInit::BoolLiteral;
        if !widen {
            if let Some(ty) = kit_type {
                insertions.push(kit_type_insertion(&d, &ty, is_ts));
            }
            continue;
        }
        let assertion = format!(";{} = __svn_any({});", d.name, d.name);
        match kit_type {
            // `let data;` — the type annotation and the assertion share
            // one ignore region after the name.
            Some(ty) if d.init == DeclaratorInit::Absent && !d.has_type_annotation => {
                if is_ts {
                    insertions.push((
                        d.list_end,
                        format!("/*svn:ignore_start*/: {ty}{assertion}/*svn:ignore_end*/"),
                    ));
                } else {
                    insertions.push((d.name_start, format!("/** @type {{{ty}}} */ ")));
                    insertions.push((
                        d.list_end,
                        format!("/*svn:ignore_start*/{assertion}/*svn:ignore_end*/"),
                    ));
                }
            }
            Some(ty) => {
                insertions.push(kit_type_insertion(&d, &ty, is_ts));
                insertions.push((
                    d.list_end,
                    format!("/*svn:ignore_start*/{assertion}/*svn:ignore_end*/"),
                ));
            }
            None => insertions.push((
                d.list_end,
                format!("/*svn:ignore_start*/{assertion}/*svn:ignore_end*/"),
            )),
        }
    }
    // Several declarators of one statement all append at its end;
    // keep declaration order within the same position.
    insertions.sort_by_key(|(pos, _)| *pos);
    splice_insertions(out, &insertions)
}

/// Give an exported `const snapshot` on a SvelteKit page or layout
/// the `Snapshot` type from its `$types` when it declares no type of
/// its own — upstream `ExportedNames.handleVariableStatement`, which
/// does this for `export const` statements only (`const_names` lists
/// the names those statements declare). The annotation lands like the
/// `let` props' one: after the name in a TS overlay, a JSDoc `@type`
/// before it in a JS one.
pub(crate) fn annotate_exported_kit_consts_in_place(
    out: &mut String,
    body: &Range<usize>,
    const_names: &[SmolStr],
    route_kind: Option<sveltekit::RouteKind>,
    is_ts: bool,
) -> Vec<(u32, u32)> {
    if !matches!(
        route_kind,
        Some(sveltekit::RouteKind::Page | sveltekit::RouteKind::Layout)
    ) || !is_target(const_names, "snapshot")
    {
        return Vec::new();
    }
    let insertions: Vec<(usize, String)> =
        collect_top_level_declarators(out, body, oxc_ast::ast::VariableDeclarationKind::Const)
            .into_iter()
            .filter(|d| {
                d.name == "snapshot"
                    && !d.has_type_annotation
                    && !d.has_jsdoc_type
                    && is_target(const_names, &d.name)
            })
            .map(|d| kit_type_insertion(&d, "import('./$types.js').Snapshot", is_ts))
            .collect();
    splice_insertions(out, &insertions)
}

/// Upstream `ExportedNames.emitKitType`: the `$types` annotation for
/// a route prop, as a TS annotation after the name (in ignore
/// comments) or a JSDoc `@type` before it.
fn kit_type_insertion(d: &LetDeclarator, ty: &str, is_ts: bool) -> (usize, String) {
    if is_ts {
        (
            d.name_end,
            format!("/*svn:ignore_start*/: {ty}/*svn:ignore_end*/"),
        )
    } else {
        (d.name_start, format!("/** @type {{{ty}}} */ "))
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::is_runes_mode;

    fn runes(source: &str) -> bool {
        let (doc, _) = svn_parser::parse_sections(source);
        let (fragment, _) = svn_parser::parse_all_template_runs(source, &doc.template.text_runs);
        let alloc = oxc_allocator::Allocator::default();
        let instance = doc
            .instance_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        let module = doc
            .module_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        is_runes_mode(&doc, &fragment, instance.as_ref(), module.as_ref())
    }

    #[test]
    fn plain_rune_call_flips_runes_mode() {
        assert!(runes("<script>let x = $state(0);</script>"));
        assert!(runes("<script>let x = $derived.by(() => 1);</script>"));
    }

    #[test]
    fn member_call_rune_names_stay_legacy() {
        // A store lib method literally named `$state` is a property
        // access, not a rune — the component keeps its Svelte-4
        // default-export shape and events strictness.
        assert!(!runes(
            "<script>export let value; const s = api.$state(0);</script>"
        ));
        assert!(!runes("<script>this.$props();</script>"));
        assert!(!runes("<script>obj?.$state(0);</script>"));
        assert!(!runes("<script>const x = my$state(0);</script>"));
    }

    #[test]
    fn options_attr_forces_runes() {
        assert!(runes("<svelte:options runes /><script>let x = 1;</script>"));
    }

    #[test]
    fn rune_in_comment_string_or_markup_is_not_a_reference() {
        // Upstream reads runes mode from the script's unresolved
        // globals, so text that only looks like a rune call is ignored.
        assert!(!runes(
            "<script>// migrate to $state(0) later\nlet x = 1;</script>"
        ));
        assert!(!runes("<script>const s = \"$state(0)\";</script>"));
        assert!(!runes(
            "<script>let v = 1;</script><p>Call $state(0) to start</p>"
        ));
    }

    #[test]
    fn inspect_alone_does_not_flip_runes_mode() {
        assert!(!runes("<script>let v = 1; $inspect(v);</script>"));
    }

    #[test]
    fn props_rune_and_top_level_await_flip_runes_mode() {
        assert!(runes("<script>let { a } = $props();</script>"));
        assert!(runes("<script>const x = await fetch('/');</script>"));
        assert!(!runes(
            "<script>async function f() { await fetch('/'); }</script>"
        ));
    }

    #[test]
    fn template_expressions_count() {
        assert!(runes(
            "<script>let n = 1;</script>{#each [n] as x}{let y = $state(x)}{y}{/each}"
        ));
        assert!(runes(
            "<script>const p = Promise.resolve(1);</script><p>{await p}</p>"
        ));
        assert!(!runes(
            "<script>const f = async () => 1;</script><button onclick={async () => { await f(); }}>x</button>"
        ));
    }

    #[test]
    fn store_named_state_is_not_a_rune() {
        assert!(!runes(
            "<script>import { writable } from 'svelte/store'; const state = writable(0); $state.set(1);</script>"
        ));
    }

    #[test]
    fn marker_less_source_stays_legacy() {
        assert!(!runes(
            "<script>export let value: string; let total = 0;</script>\n<p>{value} {total}</p>"
        ));
    }
}
