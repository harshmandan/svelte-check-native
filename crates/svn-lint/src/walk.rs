//! Top-level walker stub.
//!
//! Connects the template AST from `svn-parser` to the lint rule
//! modules. Initial scaffold: just walks elements + components. Each
//! Phase expands this.

use std::path::Path;

use smol_str::SmolStr;

use svn_parser::ast::{Attribute, Fragment, Node, SvelteElementKind};
use svn_parser::{parse_all_template_runs, parse_script_body, parse_sections};

use crate::codes::Code;
use crate::context::{CustomElementInfo, LintContext};
use crate::messages;

/// Runes-mode resolution (see `walk`, which calls these on the
/// `Document` it already parsed). Upstream heuristic:
/// - `.svelte.js` / `.svelte.ts` → runes mode (`runes_from_filename`)
/// - `<svelte:options runes={…}>` → explicit override (resolved later
///   in the template walk, in `walk`)
/// - Any rune CALL (`$state(…)`, `$derived(…)`, …) in a script body
///   → runes mode (`scripts_signal_runes`)
///
/// The call-shape check is critical: a bare substring match for
/// `$props` (etc.) false-positives on Svelte-4 ambients like
/// `$$props.class` (the legacy rest-props store). Runes are always
/// called, so requiring `(` immediately after the name excludes the
/// ambient-store pattern without needing a full parse.
///
/// `$state.raw(…)`, `$state.link(…)`, `$derived.by(…)` are also
/// call-forms; the `.` between name and `(` means a simple `rune(`
/// check would miss them. Covered by allowing optional `.WORD` before
/// the paren.
///
/// Runes-mode shortcut from the filename alone: `.svelte.js` /
/// `.svelte.ts` modules are always runes mode. No parse needed.
fn runes_from_filename(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".svelte.js") || name.ends_with(".svelte.ts")
}

/// Runes seed for the first scope-tree build: the compiler's rule
/// (any rune name referenced, or an `await` outside a function) read
/// from the parsed scripts. The scope tree recomputes the answer from
/// its own reference set below and rebuilds once if this seed was
/// wrong, so the seed only decides how much work the first build is.
fn scripts_signal_runes(
    module: Option<&oxc_ast::ast::Program<'_>>,
    instance: Option<&oxc_ast::ast::Program<'_>>,
) -> bool {
    let mut bound = std::collections::HashSet::new();
    for program in [module, instance].into_iter().flatten() {
        svn_analyze::collect_top_level_bindings(program, &mut bound);
    }
    let mut probe = svn_analyze::RunesProbe::new(svn_analyze::RunesRule::Compiler, &bound);
    for program in [module, instance].into_iter().flatten() {
        probe.scan_program(program);
    }
    probe.found
}

/// Walk a full `.svelte` source and run every phase-enabled rule.
///
/// Template parsing happens inline via `svn-parser`. Script parsing
/// happens later (Phase A's JS-side rules need the oxc AST).
pub fn walk(source: &str, path: &Path, runes: Option<bool>, ctx: &mut LintContext<'_>) {
    let (doc, _errors) = parse_sections(source);
    let (fragment, _parse_errors) = parse_all_template_runs(source, &doc.template.text_runs);
    walk_parsed(&doc, &fragment, source, path, runes, ctx);
}

/// Walk an ALREADY-PARSED `.svelte` document — the body of [`walk`]
/// after the parse. Lets a caller that already holds `(doc, fragment)`
/// (e.g. the fused native compile-error + lint pass in the CLI) run the
/// rule set without re-parsing the source.
pub fn walk_parsed(
    doc: &svn_parser::Document<'_>,
    fragment: &Fragment,
    source: &str,
    path: &Path,
    runes: Option<bool>,
    ctx: &mut LintContext<'_>,
) {
    // Resolve runes mode. A forced mode wins outright: the
    // `<svelte:options runes={…}>` attribute, then an explicit
    // caller hint (the CLI's config `compilerOptions.runes`), then
    // the `.svelte.{js,ts}` filename shortcut (rune modules are
    // always runes). Otherwise the mode is DETECTED the way the
    // compiler does (`2-analyze/index.js:456`):
    //
    //   runes = has_await || instance.has_await
    //           || module.scope.references.keys().some(is_rune)
    //
    // i.e. a function-free `await` in the instance script or a
    // template expression, or a rune-named reference that survived
    // store-sub synthesis (a backing `state` binding turns `$state`
    // into a store subscription — such a file stays legacy even
    // though the text contains `$state(`). Both directions verified
    // against the compiler.
    //
    // The AST rune probe only seeds the FIRST scope-tree build (the
    // ignore-comment strictness and the compat-gated binding fields
    // depend on the mode); when the authoritative scope-derived answer
    // disagrees, the tree is rebuilt once under the correct mode.
    ctx.runes_option = runes;
    ctx.filename = (path.as_os_str() != "(unknown)").then(|| path.to_path_buf());
    let forced: Option<bool> = svn_parser::runes_option(fragment, source)
        .or(runes)
        .or_else(|| runes_from_filename(path).then_some(true));

    // Emission order below mirrors the compiler's pipeline (verified
    // on a mixed fixture against svelte 5.56.5): parse-time warnings
    // first, then store_rune_conflict (store-sub synthesis), then the
    // <svelte:options> attribute loop, then the module → instance →
    // template walks, then the post-walk declaration loops. The CLI
    // does not sort diagnostics, so this order is user-visible.

    // Parse each script body exactly once; the scope builder and the
    // script-AST rules below both walk the same `Program`. The
    // allocator is hoisted to this frame so the parsed ASTs outlive
    // both consumers.
    // The compiler parses every script as TypeScript when the
    // component is TypeScript, whatever each tag's own `lang`.
    let compiler_ts = crate::rules::typescript_features::compiler_parses_as_ts(source);
    let script_lang = |s: &svn_parser::ScriptSection<'_>| {
        if compiler_ts {
            svn_parser::ScriptLang::Ts
        } else {
            s.lang
        }
    };
    let script_alloc = oxc_allocator::Allocator::default();
    let parsed_module = doc
        .module_script
        .as_ref()
        .map(|s| parse_script_body(&script_alloc, s.content, script_lang(s)));
    let parsed_instance = doc
        .instance_script
        .as_ref()
        .map(|s| parse_script_body(&script_alloc, s.content, script_lang(s)));
    let module_program = parsed_module.as_ref().map(|p| &p.program);
    let instance_program = parsed_instance.as_ref().map(|p| &p.program);
    ctx.runes = forced.unwrap_or_else(|| scripts_signal_runes(module_program, instance_program));

    // Build the scope tree once; Phase-C rules query it by binding
    // name from both the script walker and the template walker. The
    // template walk here is what surfaces "identifier is referenced
    // in the template, not just in a script helper" to rules like
    // `non_reactive_update`.
    let mut tree = crate::scope::build_with_template_and_runes(
        doc,
        Some(fragment),
        source,
        ctx.runes,
        ctx.compat,
        ctx.ts_scripts_transpiled,
        module_program,
        instance_program,
    );

    // Authoritative runes resolution (see the comment above). The
    // reference set and await flag are mode-independent, so the
    // preliminary build answers correctly; only the mode-dependent
    // tree state (ignore parsing, compat gates, non-runes export
    // promotion) needs the rebuild when the answer flips.
    if forced.is_none() {
        let authoritative = tree.has_await
            || tree
                .unresolved_refs
                .iter()
                .any(|r| crate::scope::is_rune_name(&r.name));
        if authoritative != ctx.runes {
            ctx.runes = authoritative;
            tree = crate::scope::build_with_template_and_runes(
                doc,
                Some(fragment),
                source,
                ctx.runes,
                ctx.compat,
                ctx.ts_scripts_transpiled,
                module_program,
                instance_program,
            );
        }
    }
    // Script-AST rule events (perf_avoid_inline_class, bidi, …) were
    // buffered by the scope build's rule hooks — the retained tree is
    // always the one built under the FINAL runes mode, so the buffer
    // matches `ctx.runes`. Flushed below, between the options
    // warnings and the walk-time binding rules.
    let script_rule_events = std::mem::take(&mut tree.script_rule_events);
    ctx.pending_template_events = std::mem::take(&mut tree.template_rule_events).into();
    let declaration_error = tree.declaration_error.take();
    ctx.scope_tree = Some(tree);

    // A template the compiler's `parse()` rejects fails before any
    // analysis.
    if let Some(finding) = crate::parse_errors::first_template_parse_error(
        fragment,
        source,
        doc.script_lang() == svn_parser::ScriptLang::Ts,
    ) {
        ctx.emit_error(finding.code, finding.message, finding.range);
    }

    // The compiler parses each script while it reads the component, so
    // a syntax error in one precedes everything the analysis reports.
    {
        use crate::rules::js_parse_error::{Script, Settings, script_parse_error};
        let scripts: Vec<Script<'_, '_, '_>> = [
            (doc.module_script.as_ref(), parsed_module.as_ref()),
            (doc.instance_script.as_ref(), parsed_instance.as_ref()),
        ]
        .into_iter()
        .filter_map(|(section, parsed)| {
            let (section, parsed) = (section?, parsed?);
            Some(Script {
                section,
                program: &parsed.program,
                errors: &parsed.errors,
                panicked: parsed.panicked,
            })
        })
        .collect();
        let settings = Settings {
            ts: compiler_ts,
            preprocess_ts: ctx.ts_scripts_transpiled,
            preprocess_configured: ctx.preprocess_configured,
        };
        if let Some((message, range)) = script_parse_error(doc, source, &scripts, &settings) {
            ctx.emit_error(Code::js_parse_error, message, range);
        }
    }

    // Stripping TypeScript happens before analysis, so its failure
    // precedes everything else.
    if compiler_ts {
        typescript_feature_check(doc, fragment, source, module_program, instance_program, ctx);
    }

    // An invalid `$` name raised while the compiler builds its scopes
    // precedes every analysis diagnostic.
    if let Some((code, message, range)) = declaration_error {
        ctx.emit_error(code, message, range);
    }

    // <script>-attribute rules (script_unknown_attribute is
    // parse-time upstream; script_context_deprecated fires early in
    // analyze — both precede the walks). Runs after the runes
    // resolution above because `script_context_deprecated` is gated
    // on the FINAL mode; the tree build emits nothing, so these
    // still lead the output.
    crate::rules::script_rules::visit_document(doc, ctx);

    // element_implicitly_closed — parse-time upstream, so it leads
    // everything the analyze phase produces.
    crate::rules::implicit_close::scan(source, ctx);

    // store_rune_conflict — upstream fires it from the store-sub
    // synthesis loop, before even the options warnings.
    crate::rules::binding_rules::visit_pre_options(ctx);

    // `<svelte:options>` attribute warnings. Mirrors the loop over
    // `root.options.attributes` in upstream's analyze phase (before
    // the walks), which fires per attribute in source order:
    //   - `accessors` / `immutable` are deprecated no-ops in runes
    //     mode (`options_deprecated_accessors` / `_immutable`);
    //   - `customElement` without the `customElement: true` compile
    //     option fires `options_missing_custom_element` and drives
    //     `custom_element_props_identifier` (fires in `binding_rules`
    //     per $props() identifier/rest candidate, via the
    //     `VariableDeclarator.js` path).
    // We don't receive compile options, so `custom_element_from_option`
    // is always false and the attribute's presence alone triggers the
    // missing-option warning. `tag-custom-element-options-true` sets
    // `customElement: true` via `_config.js`; `upstream_validator`
    // already skips that fixture via the `_config.js` escape.
    visit_svelte_options_attributes(fragment, source, ctx);

    // <script>-body (JS/TS AST) rules: perf_avoid_inline_class,
    // perf_avoid_nested_class, reactive_declaration_invalid_placement,
    // ... — buffered during the shared script walk (module first,
    // then instance — upstream walk order), replayed here where
    // upstream's analyze pipeline surfaces them: after the options
    // warnings, before the walk-time binding rules.
    crate::rules::script_ast_rules::flush(script_rule_events, ctx);

    // Walk-time binding rules (state_referenced_locally, …) —
    // upstream fires these during the instance walk, so they land
    // between the script-AST rules and the template warnings.
    crate::rules::binding_rules::visit(ctx);

    let mut ancestors: Vec<Ancestor> = Vec::new();
    walk_fragment_impl(fragment, ctx, None, &mut ancestors);
    crate::rules::binding_rules::flush_template_write_violations(ctx, u32::MAX, true);
    crate::rules::script_ast_rules::flush_template_events_before(u32::MAX, ctx);

    // Post-walk declaration loops (non_reactive_update /
    // export_let_unused) — upstream runs them after all three walks.
    crate::rules::binding_rules::visit_post_template(ctx);

    // Once every walk is done, the compiler rejects a component that
    // mixes `on:` directives and `on*` attributes on its elements,
    // pointing at the first directive.
    if ctx.uses_event_attributes
        && let Some((name, range)) = ctx.event_directive.clone()
    {
        ctx.emit_error(
            Code::mixed_event_handler_syntaxes,
            messages::mixed_event_handler_syntaxes(&name),
            range,
        );
    }

    // A component may not use both `{@render}` tags and slots: a
    // `<slot>` element (custom-element components excepted) or a
    // `$$slots` reference. The error points at the first slot name's
    // `<slot>`, or else at the first `$$slot` text in the source.
    if ctx.uses_render_tags {
        let uses_slots = ctx
            .scope_tree
            .as_ref()
            .is_some_and(|tree| tree.unresolved_refs.iter().any(|r| r.name == "$$slots"));
        let uses_slot_elements = ctx.first_slot.is_some() && ctx.custom_element_info.is_none();
        if uses_slots || uses_slot_elements {
            let range = match ctx.first_slot.as_ref() {
                Some((_, range)) => *range,
                None => {
                    let at = crate::rules::transpile_positions::first_dollar_slot(
                        doc,
                        source,
                        module_program,
                        instance_program,
                        ctx.ts_scripts_transpiled,
                    );
                    svn_core::Range::new(at, at)
                }
            };
            ctx.emit_error(
                Code::slot_snippet_conflict,
                messages::slot_snippet_conflict(),
                range,
            );
        }
    }
}

/// `typescript_invalid_feature` for a TypeScript component: the
/// template, then the instance script, then the module script, as the
/// compiler strips them. A `<script lang="ts">` the project's
/// preprocessors transpile is checked for what the transpiled code
/// keeps.
fn typescript_feature_check(
    doc: &svn_parser::Document<'_>,
    fragment: &Fragment,
    source: &str,
    module_program: Option<&oxc_ast::ast::Program<'_>>,
    instance_program: Option<&oxc_ast::ast::Program<'_>>,
    ctx: &mut LintContext<'_>,
) {
    use crate::rules::typescript_features::{Finding, first_finding, first_template_finding};
    let preprocess_ts = ctx.ts_scripts_transpiled;
    let transpiled = |s: &svn_parser::ScriptSection<'_>| {
        crate::rules::typescript_features::script_is_transpiled(s, preprocess_ts)
    };
    let script_finding = |s: Option<&svn_parser::ScriptSection<'_>>,
                          program: Option<&oxc_ast::ast::Program<'_>>| {
        let (s, program) = (s?, program?);
        first_finding(program, s.content_range.start, transpiled(s))
    };
    let finding = first_template_finding(fragment, source)
        .or_else(|| script_finding(doc.instance_script.as_ref(), instance_program))
        .or_else(|| script_finding(doc.module_script.as_ref(), module_program));
    match finding {
        Some(Finding::Invalid { feature, range }) => ctx.emit_error(
            Code::typescript_invalid_feature,
            messages::typescript_invalid_feature(feature),
            range,
        ),
        Some(Finding::Crash) => ctx.abort(),
        None => {}
    }
}

/// Scan the top-level fragment for `<svelte:options>` and fire the
/// per-attribute warnings in source order, mirroring upstream's
/// `for (const attribute of root.options.attributes)` loop:
/// `accessors` / `immutable` warn (in runes mode only) that the option
/// is a deprecated no-op; `customElement` warns that the compile
/// option is missing and records [`CustomElementInfo`]. Each warning
/// spans the whole attribute (upstream passes the attribute node).
/// The name check is name-only — the attribute's value shape and
/// truthiness are irrelevant, so `accessors={false}` still warns.
fn visit_svelte_options_attributes(fragment: &Fragment, source: &str, ctx: &mut LintContext<'_>) {
    for node in &fragment.nodes {
        let Node::SvelteElement(se) = node else {
            continue;
        };
        if se.kind != SvelteElementKind::Options {
            continue;
        }
        for attr in &se.attributes {
            let (attr_name, attr_range) = match attr {
                Attribute::Plain(p) => (p.name.as_str(), p.range),
                Attribute::Expression(e) => (e.name.as_str(), e.range),
                Attribute::Shorthand(s) => (s.name.as_str(), s.range),
                _ => continue,
            };
            match attr_name {
                "accessors" if ctx.runes => {
                    ctx.emit(
                        Code::options_deprecated_accessors,
                        messages::options_deprecated_accessors(),
                        attr_range,
                    );
                }
                "immutable" if ctx.runes => {
                    ctx.emit(
                        Code::options_deprecated_immutable,
                        messages::options_deprecated_immutable(),
                        attr_range,
                    );
                }
                "customElement" => {
                    // Whether the literal object value has a `props`
                    // key — only the `customElement={{...}}` object
                    // form can carry one; string / boolean / shorthand
                    // forms have no props option.
                    let has_props_option = match attr {
                        Attribute::Expression(e) => source
                            .get(e.expression_range.start as usize..e.expression_range.end as usize)
                            .map(object_expression_has_props_key)
                            .unwrap_or(false),
                        _ => false,
                    };
                    ctx.emit(
                        Code::options_missing_custom_element,
                        messages::options_missing_custom_element(),
                        attr_range,
                    );
                    ctx.custom_element_info = Some(CustomElementInfo { has_props_option });
                }
                _ => {}
            }
        }
    }
}

/// Parse `expr` as a JS expression and return true iff it's an
/// ObjectExpression with a `props` key (identifier or string literal).
/// Upstream `VariableDeclarator.js:74` reads
/// `options.customElementOptions?.props`; that's extracted during
/// validate-options from the same object literal when the
/// svelte:options attribute is an ObjectExpression. Non-object
/// expressions (e.g. a variable reference) still mean "custom element
/// opts", but they carry no props option.
fn object_expression_has_props_key(src: &str) -> bool {
    let alloc = oxc_allocator::Allocator::default();
    // Wrap in parens so a bare `{...}` is parsed as an
    // ObjectExpression, not a BlockStatement.
    let wrapped = format!("({})", src.trim());
    let parser = oxc_parser::Parser::new(&alloc, &wrapped, oxc_span::SourceType::mjs());
    let parsed = parser.parse();
    // Ignore parse errors — an unparseable expression simply can't
    // carry a `props` key we'd trust, so treat as absent.
    let Some(stmt) = parsed.program.body.first() else {
        return false;
    };
    let oxc_ast::ast::Statement::ExpressionStatement(e) = stmt else {
        return false;
    };
    let inner = match &e.expression {
        oxc_ast::ast::Expression::ParenthesizedExpression(p) => &p.expression,
        other => other,
    };
    let oxc_ast::ast::Expression::ObjectExpression(obj) = inner else {
        return false;
    };
    obj.properties.iter().any(|p| match p {
        oxc_ast::ast::ObjectPropertyKind::ObjectProperty(prop) => match &prop.key {
            oxc_ast::ast::PropertyKey::StaticIdentifier(id) => id.name.as_str() == "props",
            oxc_ast::ast::PropertyKey::StringLiteral(s) => s.value.as_str() == "props",
            _ => false,
        },
        _ => false,
    })
}

/// One frame of the enclosing-node stack threaded through the
/// template walk. Mirrors the slice of upstream's `context.path` that
/// the ancestor-driven rules inspect: upstream's path never resets, so
/// consumers see every enclosing node and apply their own skip/stop
/// rules per node type.
///
/// Two consumers, two semantics:
/// - a11y `is_parent` (autofocus-in-dialog, figcaption-in-figure,
///   redundant header/footer roles) walks past `Boundary` frames and
///   treats a `SvelteElement` frame as "unknown, play it safe" (true).
/// - HTML tree-model placement checks stop at the first non-`Element`
///   frame, exactly like upstream RegularElement.js breaks its
///   ancestor scan at Component / SvelteElement / SnippetBlock.
#[derive(Debug, Clone)]
pub(crate) enum Ancestor {
    /// A regular DOM element, carrying its tag name.
    Element(String),
    /// A `<svelte:element>` — renders as an unknown tag.
    SvelteElement,
    /// A Component or `{#snippet}` frame.
    Boundary,
}

/// One enclosing template node, as the compiler's validations see it
/// in `context.path`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum PathFrame {
    IfBlock,
    /// `has_key` records a `(key)` expression; `body_nodes` counts the
    /// body's children other than comments, `{@const}` tags and
    /// whitespace-only text — the inputs of the `animate:` placement
    /// rules.
    EachBlock {
        has_key: bool,
        body_nodes: usize,
    },
    AwaitBlock,
    KeyBlock,
    SnippetBlock,
    /// `<Component>` / `<svelte:component>` / `<svelte:self>`.
    /// `implicit_children` is set when the component has children
    /// other than snippets, comments and whitespace.
    Component {
        kind: ComponentKind,
        /// The tag name (`svelte:component` / `svelte:self` for those).
        name: SmolStr,
        implicit_children: bool,
        /// The first child that counts as default-slot content.
        default_slot_content: Option<svn_core::Range>,
        /// The slot names its direct children have filled so far.
        filled_slots: Vec<SmolStr>,
    },
    /// `<svelte:element>`; `slotted` marks one carrying a `slot`
    /// attribute.
    SvelteElement {
        slotted: bool,
    },
    /// A regular DOM element; `custom` marks a custom element (a
    /// hyphenated name or an `is` attribute), `slotted` one carrying a
    /// `slot` attribute.
    RegularElement {
        name: SmolStr,
        custom: bool,
        slotted: bool,
    },
    /// `<svelte:fragment>`.
    SvelteFragment,
    /// `<svelte:boundary>`.
    SvelteBoundary,
    /// `<svelte:head>`.
    SvelteHead,
    /// Any other element-like node (`<slot>`, `<svelte:head>`, …).
    Other,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ComponentKind {
    Component,
    SvelteComponent,
    SvelteSelf,
}

impl PathFrame {
    pub(crate) fn is_block(&self) -> bool {
        matches!(
            self,
            Self::IfBlock | Self::EachBlock { .. } | Self::AwaitBlock | Self::KeyBlock
        )
    }

    fn each(b: &svn_parser::ast::EachBlock, source: &str) -> Self {
        let body_nodes = b
            .body
            .nodes
            .iter()
            .filter(|n| match n {
                Node::Comment(_) => false,
                Node::Interpolation(i) => i.kind != svn_parser::InterpolationKind::AtConst,
                Node::Text(t) => !t
                    .range
                    .slice(source)
                    .chars()
                    .all(crate::rules::block_rules::is_js_trim_ws),
                _ => true,
            })
            .count();
        Self::EachBlock {
            has_key: b.as_clause.as_ref().is_some_and(|c| c.key_range.is_some()),
            body_nodes,
        }
    }

    fn component(kind: ComponentKind, name: &str, children: &Fragment, source: &str) -> Self {
        let implicit_children = children.nodes.iter().any(|n| match n {
            Node::SnippetBlock(_) | Node::Comment(_) => false,
            Node::Text(t) => !t
                .range
                .slice(source)
                .chars()
                .all(crate::rules::block_rules::is_js_trim_ws),
            _ => true,
        });
        // The first child that would be default-slot content next to an
        // explicit `slot="default"`: anything but whitespace text and
        // elements / `<svelte:fragment>`s carrying a `slot` attribute.
        let default_slot_content = children
            .nodes
            .iter()
            .find(|n| match n {
                Node::Text(t) => {
                    let text = t.range.slice(source);
                    text.is_empty()
                        || !text
                            .chars()
                            .all(|c| matches!(c, ' ' | '\t' | '\n' | '\r' | '\u{c}'))
                }
                Node::Element(el) => el.name == "slot" || !has_slot_attribute(&el.attributes),
                Node::SvelteElement(se) => {
                    !matches!(se.kind, SvelteElementKind::Fragment)
                        || !has_slot_attribute(&se.attributes)
                }
                _ => true,
            })
            .map(Node::range);
        Self::Component {
            kind,
            name: SmolStr::new(name),
            implicit_children,
            default_slot_content,
            filled_slots: Vec::new(),
        }
    }
}

/// Whether the element carries a `slot` attribute (of any value form).
pub(crate) fn has_slot_attribute(attributes: &[Attribute]) -> bool {
    attributes.iter().any(|a| match a {
        Attribute::Plain(p) => p.name == "slot",
        Attribute::Expression(e) => e.name == "slot",
        Attribute::Shorthand(s) => s.name == "slot",
        _ => false,
    })
}

/// The compiler's `is_custom_element_node`: a regular element whose
/// name contains `-` or that carries an `is` attribute.
pub(crate) fn is_custom_element_node(name: &str, attributes: &[Attribute]) -> bool {
    name.contains('-')
        || attributes.iter().any(|a| match a {
            Attribute::Plain(p) => p.name == "is",
            Attribute::Expression(e) => e.name == "is",
            Attribute::Shorthand(s) => s.name == "is",
            _ => false,
        })
}

/// Recursively visit every template node, dispatching rules as we go.
///
/// `parent_tag`: closest enclosing regular-element tag, for
/// `is_tag_valid_with_parent` checks.
/// `ancestors`: stack of enclosing nodes (outer → inner) — see
/// [`Ancestor`] for how each consumer interprets the frames.
fn walk_fragment_impl(
    fragment: &Fragment,
    ctx: &mut LintContext<'_>,
    parent_tag: Option<&str>,
    ancestors: &mut Vec<Ancestor>,
) {
    let nodes: Vec<&Node> = fragment.nodes.iter().collect();
    walk_fragment_nodes(&nodes, ctx, parent_tag, ancestors);
}

/// A component's children, visited the way the compiler's
/// `visit_component` does: as one fragment per slot they fill — the
/// default slot's first, then each named slot in order of first
/// appearance. The comments before a child go into its fragment just
/// before it; before a default-slot child they are also kept for the
/// next child, so a `svelte-ignore` there reaches every later
/// default-slot child. Trailing comments belong to no fragment.
fn walk_component_children(
    children: &Fragment,
    ctx: &mut LintContext<'_>,
    ancestors: &mut Vec<Ancestor>,
) {
    let source = ctx.source;
    let mut groups: Vec<(&str, Vec<&Node>)> = vec![("default", Vec::new())];
    let mut comments: Vec<&Node> = Vec::new();
    for node in &children.nodes {
        if matches!(node, Node::Comment(_)) {
            comments.push(node);
            continue;
        }
        let slot = filled_slot(node, source).unwrap_or("default");
        let group = match groups.iter().position(|(name, _)| *name == slot) {
            Some(i) => i,
            None => {
                groups.push((slot, Vec::new()));
                groups.len() - 1
            }
        };
        groups[group].1.extend(comments.iter().copied());
        groups[group].1.push(node);
        if slot != "default" {
            comments.clear();
        }
    }
    for (_, nodes) in groups {
        walk_fragment_nodes(&nodes, ctx, None, ancestors);
    }
}

/// The compiler's `determine_slot`: the static `slot` attribute value
/// of an element-like node.
fn filled_slot<'s>(node: &Node, source: &'s str) -> Option<&'s str> {
    let attributes = match node {
        Node::Element(el) => &el.attributes,
        Node::Component(c) => &c.attributes,
        Node::SvelteElement(se) => &se.attributes,
        _ => return None,
    };
    attributes.iter().find_map(|a| match a {
        Attribute::Plain(p) if p.name == "slot" => {
            let value = p.value.as_ref()?;
            match value.parts.as_slice() {
                [] if value.quoted => Some(""),
                [svn_parser::ast::AttrValuePart::Text { range }] => {
                    source.get(range.start as usize..range.end as usize)
                }
                _ => None,
            }
        }
        _ => None,
    })
}

/// Visit the nodes of one fragment in order; `nodes` is the fragment
/// as the compiler's visitors see it, whose earlier entries are each
/// node's preceding siblings.
fn walk_fragment_nodes(
    nodes: &[&Node],
    ctx: &mut LintContext<'_>,
    parent_tag: Option<&str>,
    ancestors: &mut Vec<Ancestor>,
) {
    let source = ctx.source;
    for (idx, &node) in nodes.iter().enumerate() {
        crate::rules::binding_rules::flush_template_write_violations(
            ctx,
            node.range().start,
            false,
        );
        // Ignore-stack: pull any svelte-ignore comments immediately
        // preceding this node (in the same fragment). These scope
        // the ignore to this one node and its subtree — mirror
        // upstream `_()` catchall visitor.
        //
        // Every node but a comment or a text node consumes the ignore
        // comments right before it, as the compiler's catch-all visitor
        // does (`node.type !== 'Comment' && node.type !== 'Text'`). A
        // comment or text consuming them would report each comment's
        // `legacy_code` / `unknown_code` a second time. Text nodes run
        // their own comment scan for the bidi warning
        // (`text_rules::visit_text`).
        let is_target = match node {
            Node::Element(_)
            | Node::Component(_)
            | Node::SvelteElement(_)
            | Node::IfBlock(_)
            | Node::EachBlock(_)
            | Node::AwaitBlock(_)
            | Node::KeyBlock(_)
            | Node::SnippetBlock(_)
            // `{expr}`, `{@html}`, `{@render}`, `{@const}` and `{@debug}`
            // consume a preceding ignore comment too.
            | Node::Interpolation(_) => true,
            Node::Text(_) | Node::Comment(_) => false,
        };
        let ignores = if is_target {
            crate::ignore::collect_preceding_comment_ignores(nodes, idx, ctx)
        } else {
            Vec::new()
        };
        let pushed = !ignores.is_empty();
        if pushed {
            ctx.push_ignore(ignores);
        }

        match node {
            Node::Element(el) => {
                crate::rules::element_rules::visit(el, ctx, parent_tag, ancestors);
                attribute_text(&el.attributes, ctx);
                flush_expr_events_before(children_start(&el.children, el.range), ctx);
                ancestors.push(Ancestor::Element(el.name.to_string()));
                // `<slot>` is a SlotElement to the compiler: it neither
                // becomes its children's parent element nor counts as a
                // regular-element ancestor.
                let is_slot = el.name == "slot";
                ctx.template_path.push(if is_slot {
                    PathFrame::Other
                } else {
                    PathFrame::RegularElement {
                        name: el.name.clone(),
                        custom: is_custom_element_node(&el.name, &el.attributes),
                        slotted: has_slot_attribute(&el.attributes),
                    }
                });
                walk_fragment_impl(
                    &el.children,
                    ctx,
                    if is_slot {
                        parent_tag
                    } else {
                        Some(el.name.as_str())
                    },
                    ancestors,
                );
                ctx.template_path.pop();
                ancestors.pop();
            }
            Node::Component(comp) => {
                crate::rules::component_rules::visit(comp, ctx);
                attribute_text(&comp.attributes, ctx);
                flush_expr_events_before(children_start(&comp.children, comp.range), ctx);
                // A Boundary frame: the HTML placement checks stop
                // here (upstream RegularElement.js breaks at a
                // Component ancestor), but the a11y is_parent walk
                // continues past it — upstream's path never resets.
                ancestors.push(Ancestor::Boundary);
                ctx.template_path.push(PathFrame::component(
                    ComponentKind::Component,
                    &comp.name,
                    &comp.children,
                    source,
                ));
                walk_component_children(&comp.children, ctx, ancestors);
                ctx.template_path.pop();
                ancestors.pop();
            }
            Node::SvelteElement(se) => {
                crate::rules::svelte_element_rules::visit(se, ctx, ancestors);
                // `<svelte:options>` is lifted out of the template.
                if se.kind != SvelteElementKind::Options {
                    attribute_text(&se.attributes, ctx);
                }
                flush_expr_events_before(children_start(&se.children, se.range), ctx);
                // Placement checks stop here too, while the a11y
                // is_parent walk answers "unknown tag — play it safe"
                // for this frame.
                ancestors.push(Ancestor::SvelteElement);
                let (frame, child_parent_tag) = match se.kind {
                    SvelteElementKind::Component => (
                        PathFrame::component(
                            ComponentKind::SvelteComponent,
                            "svelte:component",
                            &se.children,
                            source,
                        ),
                        None,
                    ),
                    SvelteElementKind::SelfRef => (
                        PathFrame::component(
                            ComponentKind::SvelteSelf,
                            "svelte:self",
                            &se.children,
                            source,
                        ),
                        None,
                    ),
                    SvelteElementKind::Element => (
                        PathFrame::SvelteElement {
                            slotted: has_slot_attribute(&se.attributes),
                        },
                        None,
                    ),
                    SvelteElementKind::Fragment => (PathFrame::SvelteFragment, None),
                    SvelteElementKind::Boundary => (PathFrame::SvelteBoundary, parent_tag),
                    // The remaining special elements have no visitor of
                    // their own: their children keep the parent element.
                    SvelteElementKind::Head => (PathFrame::SvelteHead, parent_tag),
                    SvelteElementKind::Window
                    | SvelteElementKind::Document
                    | SvelteElementKind::Body
                    | SvelteElementKind::Options => (PathFrame::Other, parent_tag),
                };
                let is_component = matches!(frame, PathFrame::Component { .. });
                ctx.template_path.push(frame);
                if is_component {
                    walk_component_children(&se.children, ctx, ancestors);
                } else {
                    walk_fragment_impl(&se.children, ctx, child_parent_tag, ancestors);
                }
                ctx.template_path.pop();
                ancestors.pop();
            }
            Node::IfBlock(b) => {
                crate::rules::block_rules::visit_if(b, ctx);
                flush_expr_events_before(b.consequent.range.start, ctx);
                ctx.template_path.push(PathFrame::IfBlock);
                walk_fragment_impl(&b.consequent, ctx, parent_tag, ancestors);
                for arm in &b.elseif_arms {
                    crate::rules::block_rules::visit_elseif(arm, ctx);
                    flush_expr_events_before(arm.body.range.start, ctx);
                    walk_fragment_impl(&arm.body, ctx, parent_tag, ancestors);
                }
                if let Some(else_body) = &b.alternate {
                    walk_fragment_impl(else_body, ctx, parent_tag, ancestors);
                }
                ctx.template_path.pop();
            }
            Node::EachBlock(b) => {
                crate::rules::block_rules::visit_each(b, ctx);
                flush_expr_events_before(b.body.range.start, ctx);
                ctx.template_path.push(PathFrame::each(b, source));
                walk_fragment_impl(&b.body, ctx, parent_tag, ancestors);
                if let Some(empty) = &b.alternate {
                    walk_fragment_impl(empty, ctx, parent_tag, ancestors);
                }
                ctx.template_path.pop();
            }
            Node::AwaitBlock(b) => {
                crate::rules::block_rules::visit_await(b, ctx);
                let first_body = b
                    .pending
                    .as_ref()
                    .map(|p| p.range.start)
                    .or(b.then_branch.as_ref().map(|t| t.body.range.start))
                    .or(b.catch_branch.as_ref().map(|c| c.body.range.start))
                    .unwrap_or(b.range.end);
                flush_expr_events_before(first_body, ctx);
                ctx.template_path.push(PathFrame::AwaitBlock);
                if let Some(pending) = &b.pending {
                    walk_fragment_impl(pending, ctx, parent_tag, ancestors);
                }
                if let Some(then) = &b.then_branch {
                    walk_fragment_impl(&then.body, ctx, parent_tag, ancestors);
                }
                if let Some(catch) = &b.catch_branch {
                    walk_fragment_impl(&catch.body, ctx, parent_tag, ancestors);
                }
                ctx.template_path.pop();
            }
            Node::KeyBlock(b) => {
                crate::rules::block_rules::visit_key(b, ctx);
                flush_expr_events_before(b.body.range.start, ctx);
                ctx.template_path.push(PathFrame::KeyBlock);
                walk_fragment_impl(&b.body, ctx, parent_tag, ancestors);
                ctx.template_path.pop();
            }
            Node::SnippetBlock(b) => {
                // Snippet frames stop the placement checks (upstream
                // breaks at SnippetBlock) but not the a11y is_parent
                // walk.
                crate::rules::block_rules::visit_snippet(b, ctx);
                ancestors.push(Ancestor::Boundary);
                ctx.template_path.push(PathFrame::SnippetBlock);
                // The compiler clears the parent element for a
                // snippet's body.
                walk_fragment_impl(&b.body, ctx, None, ancestors);
                ctx.template_path.pop();
                ancestors.pop();
                crate::rules::block_rules::visit_snippet_after_body(b, ctx);
            }
            Node::Text(t) => {
                // `regex_not_whitespace`: anything but space, tab, CR, LF.
                if let Some(parent) = parent_tag
                    && t.range
                        .slice(source)
                        .chars()
                        .any(|c| !matches!(c, ' ' | '\t' | '\r' | '\n'))
                {
                    text_placement_error(parent, t.range, ctx);
                }
                crate::rules::text_rules::visit_text(t, &nodes[..idx], ctx);
            }
            Node::Interpolation(i) => {
                if i.kind == svn_parser::InterpolationKind::AtRender {
                    crate::rules::block_rules::visit_render_tag(i, ctx);
                    ctx.uses_render_tags = true;
                }
                if i.kind == svn_parser::InterpolationKind::AtConst {
                    crate::rules::block_rules::visit_const_tag(i, ctx);
                }
                if matches!(
                    i.kind,
                    svn_parser::InterpolationKind::AtHtml | svn_parser::InterpolationKind::AtDebug
                ) {
                    crate::rules::block_rules::visit_html_or_debug_tag(i, ctx);
                }
                if i.kind == svn_parser::InterpolationKind::Expression
                    && let Some(parent) = parent_tag
                {
                    text_placement_error(parent, i.range, ctx);
                }
            }
            Node::Comment(_) => {}
        }
        flush_expr_events_before(node_end(node), ctx);

        if pushed {
            ctx.pop_ignore();
        }
    }
}

/// `node_invalid_placement` for text content (a Text node or an
/// `{expression}` tag) whose parent element does not allow text
/// (`Text.js` / `ExpressionTag.js`).
fn text_placement_error(parent: &str, range: svn_core::Range, ctx: &mut LintContext<'_>) {
    if let Some(msg) = crate::html5::is_tag_valid_with_parent("#text", parent) {
        ctx.emit_error(
            Code::node_invalid_placement,
            messages::node_invalid_placement(&msg),
            range,
        );
    }
}

/// The compiler visits an element's attributes after its own checks:
/// value text chunks run the text visitor, interleaved with the
/// expressions' events by position.
fn attribute_text(attributes: &[Attribute], ctx: &mut LintContext<'_>) {
    use svn_parser::ast::{AttrValuePart, DirectiveValue};
    for attr in attributes {
        let parts = match attr {
            Attribute::Plain(p) => p.value.as_ref().map(|v| v.parts.as_slice()),
            Attribute::Directive(d) => match &d.value {
                Some(DirectiveValue::Quoted(v)) => Some(v.parts.as_slice()),
                _ => None,
            },
            Attribute::Expression(_)
            | Attribute::Shorthand(_)
            | Attribute::Spread(_)
            | Attribute::Comment(_) => None,
        };
        for part in parts.unwrap_or_default() {
            if let AttrValuePart::Text { range } = part {
                flush_expr_events_before(range.start, ctx);
                crate::rules::text_rules::visit_attribute_text(*range, ctx);
            }
        }
    }
}

fn flush_expr_events_before(until: u32, ctx: &mut LintContext<'_>) {
    crate::rules::script_ast_rules::flush_template_events_before(until, ctx);
}

/// Where an element-like node's children begin — its attribute
/// expressions all precede this point. A childless node's expressions
/// all precede its end.
fn children_start(children: &Fragment, node_range: svn_core::Range) -> u32 {
    if children.nodes.is_empty() {
        node_range.end
    } else {
        children.range.start
    }
}

fn node_end(node: &Node) -> u32 {
    match node {
        Node::Element(n) => n.range.end,
        Node::Component(n) => n.range.end,
        Node::SvelteElement(n) => n.range.end,
        Node::IfBlock(n) => n.range.end,
        Node::EachBlock(n) => n.range.end,
        Node::AwaitBlock(n) => n.range.end,
        Node::KeyBlock(n) => n.range.end,
        Node::SnippetBlock(n) => n.range.end,
        Node::Text(n) => n.range.end,
        Node::Interpolation(n) => n.range.end,
        Node::Comment(n) => n.range.end,
    }
}

#[cfg(test)]
mod runes_inference_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use std::path::PathBuf;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
    }

    /// Exercises the same decomposition `walk()` uses (filename
    /// shortcut, then a rune-call scan over the parsed document), so
    /// these tests cover the production runes-resolution path.
    fn infer_runes_mode(source: &str, path: &std::path::Path) -> bool {
        if super::runes_from_filename(path) {
            return true;
        }
        let (doc, _) = svn_parser::parse_sections(source);
        let alloc = oxc_allocator::Allocator::default();
        let module = doc
            .module_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        let instance = doc
            .instance_script
            .as_ref()
            .map(|s| svn_parser::parse_script_body(&alloc, s.content, s.lang));
        super::scripts_signal_runes(
            module.as_ref().map(|p| &p.program),
            instance.as_ref().map(|p| &p.program),
        )
    }

    #[test]
    fn rune_call_in_instance_script_enables_runes() {
        let src = "<script>let count = $state(0);</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_inside_line_comment_does_not_enable_runes() {
        let src = "<script>\n// example: let x = $state(0);\nlet y = 1;\n</script>";
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_inside_block_comment_does_not_enable_runes() {
        let src = "<script>/* let x = $state(1); */ let y = 1;</script>";
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_inside_string_does_not_enable_runes() {
        let src = r#"<script>let x = "$state(1)"; let y = '$derived(2)';</script>"#;
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_inside_template_literal_text_does_not_enable_runes() {
        let src = "<script>const docs = `use $state(value) here`;</script>";
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_inside_template_interpolation_enables_runes() {
        // The interpolation IS code — a rune call there is real.
        let src = "<script>const x = `${$state(0)}`;</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn brace_inside_string_does_not_terminate_interpolation_early() {
        // The closing brace inside the string literal must not be
        // treated as the interpolation terminator. The previous raw
        // brace counter would have stopped at the `}` inside `"}"`,
        // missing the `$state(0)` after it.
        let src = r#"<script>const x = `${"}" + $state(0)}`;</script>"#;
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn brace_inside_block_comment_does_not_terminate_interpolation_early() {
        let src = "<script>const x = `${/* } */ $state(0)}`;</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn nested_template_interpolation_resolves_correctly() {
        let src = "<script>const x = `${`${$state(0)}`}`;</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_in_template_html_does_not_enable_runes() {
        // F12: the previous scan ran over the WHOLE Svelte source —
        // a `$state(` literal in template HTML or comment text could
        // false-positive. The new scan scopes to script bodies.
        let src = r#"<!-- example: $state(0) -->
<div>doc text: $state(0)</div>
<script>let y = 1;</script>"#;
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn dotted_rune_call_enables_runes() {
        let src = "<script>let x = $state.raw([]);</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn ambient_rest_props_does_not_enable_runes() {
        // `$$props` is the legacy rest-props ambient, not a rune.
        let src = "<script>const cls = $$props.class;</script>";
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn svelte_js_runes_module_enables_runes_unconditionally() {
        // Filename ending in .svelte.js is a Svelte-5 runes module
        // by definition; no scan needed.
        let src = "// no rune calls here";
        assert!(infer_runes_mode(src, &p("foo.svelte.js")));
    }

    #[test]
    fn rune_call_inside_regex_literal_does_not_enable_runes() {
        // A regex literal is not code — `/\$state\(/` must not flip
        // runes mode (the real compiler stays non-runes here).
        let src = r"<script>const re = /\$state\(/; void re;</script>";
        assert!(!infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_after_division_still_enables_runes() {
        // The `/` here is division, not a regex opener — the scan
        // must not swallow the rest of the script.
        let src = "<script>let n = 1 / 2;\nlet c = $state(0);\nvoid n; void c;</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }

    #[test]
    fn rune_call_after_regex_literal_still_enables_runes() {
        let src = r"<script>const re = /x/g; let c = $state(0); void re; void c;</script>";
        assert!(infer_runes_mode(src, &p("Foo.svelte")));
    }
}

#[cfg(test)]
mod runes_options_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use svn_parser::{parse_all_template_runs, parse_sections, runes_option};

    fn detect(src: &str) -> Option<bool> {
        let (doc, _) = parse_sections(src);
        let (fragment, _) = parse_all_template_runs(src, &doc.template.text_runs);
        runes_option(&fragment, src)
    }

    #[test]
    fn runes_expr_false_is_false() {
        // G1 regression: `runes={false}` was being treated as truthy,
        // so files explicitly opting out of runes mode got linted
        // under the wrong rule set.
        assert_eq!(detect("<svelte:options runes={false} />"), Some(false));
    }

    #[test]
    fn runes_expr_true_stays_true() {
        assert_eq!(detect("<svelte:options runes={true} />"), Some(true));
    }

    #[test]
    fn runes_expr_unknown_falls_back_to_true() {
        // A variable reference (`runes={x}`) can't be statically
        // resolved — fall back to truthy so we don't regress files
        // that legitimately rely on dynamic config.
        assert_eq!(detect("<svelte:options runes={x} />"), Some(true));
    }

    #[test]
    fn runes_bare_attribute_is_true() {
        assert_eq!(detect("<svelte:options runes />"), Some(true));
    }

    #[test]
    fn runes_attr_string_false_is_false() {
        assert_eq!(detect(r#"<svelte:options runes="false" />"#), Some(false));
    }
}
