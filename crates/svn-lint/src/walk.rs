//! Top-level walker stub.
//!
//! Connects the template AST from `svn-parser` to the lint rule
//! modules. Initial scaffold: just walks elements + components. Each
//! Phase expands this.

use std::path::Path;

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
    let script_alloc = oxc_allocator::Allocator::default();
    let parsed_module = doc
        .module_script
        .as_ref()
        .map(|s| parse_script_body(&script_alloc, s.content, s.lang));
    let parsed_instance = doc
        .instance_script
        .as_ref()
        .map(|s| parse_script_body(&script_alloc, s.content, s.lang));
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
    ctx.scope_tree = Some(tree);

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
    walk_fragment_impl(fragment, ctx, None, &mut ancestors, false);

    // Post-walk declaration loops (non_reactive_update /
    // export_let_unused) — upstream runs them after all three walks.
    crate::rules::binding_rules::visit_post_template(ctx);
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

/// Recursively visit every template node, dispatching rules as we go.
///
/// `parent_tag`: closest enclosing regular-element tag, for
/// `is_tag_valid_with_parent` checks.
/// `ancestors`: stack of enclosing nodes (outer → inner) — see
/// [`Ancestor`] for how each consumer interprets the frames.
/// `inside_control_block`: true if we're currently inside an
/// `{#if}`/`{#each}`/`{#await}`/`{#key}`. Only in that case does
/// the placement warning fire (otherwise upstream errors).
fn walk_fragment_impl(
    fragment: &Fragment,
    ctx: &mut LintContext<'_>,
    parent_tag: Option<&str>,
    ancestors: &mut Vec<Ancestor>,
    inside_control_block: bool,
) {
    let source = ctx.source;
    for (idx, node) in fragment.nodes.iter().enumerate() {
        // Ignore-stack: pull any svelte-ignore comments immediately
        // preceding this node (in the same fragment). These scope
        // the ignore to this one node and its subtree — mirror
        // upstream `_()` catchall visitor.
        //
        // Only lintable nodes need their own ignore frame. Comment
        // and Interpolation nodes don't emit warnings themselves AND
        // shouldn't trigger a walk-back through a `<!-- svelte-ignore
        // -->` comment — otherwise the comment would double-fire its
        // own `legacy_code` / `unknown_code` (once for the comment,
        // once for the lintable sibling that follows).
        //
        // Text nodes DO emit warnings (bidi) so they need the ignore
        // frame. Whitespace-only Text is a neutral carrier in the
        // preceding-comments chain.
        let is_target = match node {
            Node::Element(_)
            | Node::Component(_)
            | Node::SvelteElement(_)
            | Node::IfBlock(_)
            | Node::EachBlock(_)
            | Node::AwaitBlock(_)
            | Node::KeyBlock(_)
            | Node::SnippetBlock(_) => true,
            // Non-whitespace Text carries bidi warnings and needs
            // the ignore frame; whitespace-only Text is a neutral
            // carrier between the comment and its target element.
            Node::Text(t) => t.range.slice(source).chars().any(|c| !c.is_whitespace()),
            _ => false,
        };
        let ignores = if is_target {
            crate::ignore::collect_preceding_comment_ignores(&fragment.nodes, idx, ctx)
        } else {
            Vec::new()
        };
        let pushed = !ignores.is_empty();
        if pushed {
            ctx.push_ignore(ignores);
        }

        match node {
            Node::Element(el) => {
                crate::rules::element_rules::visit(
                    el,
                    ctx,
                    parent_tag,
                    ancestors,
                    inside_control_block,
                );
                ancestors.push(Ancestor::Element(el.name.to_string()));
                walk_fragment_impl(
                    &el.children,
                    ctx,
                    Some(el.name.as_str()),
                    ancestors,
                    // Reset: crossing a regular element means a control
                    // block above it no longer sits between deeper nodes
                    // and their nearest regular-element parent. This is
                    // upstream's per-node `only_warn` for the parent check
                    // (RegularElement.js:178-190) — a sticky bool fired
                    // spurious node_invalid_placement_ssr on e.g.
                    // `{#if x}<ul><p/></ul>{/if}`.
                    false,
                );
                ancestors.pop();
            }
            Node::Component(comp) => {
                crate::rules::component_rules::visit(comp, ctx);
                // A Boundary frame: the HTML placement checks stop
                // here (upstream RegularElement.js breaks at a
                // Component ancestor), but the a11y is_parent walk
                // continues past it — upstream's path never resets.
                ancestors.push(Ancestor::Boundary);
                walk_fragment_impl(&comp.children, ctx, None, ancestors, false);
                ancestors.pop();
            }
            Node::SvelteElement(se) => {
                crate::rules::svelte_element_rules::visit(se, ctx, ancestors);
                // Placement checks stop here too, while the a11y
                // is_parent walk answers "unknown tag — play it safe"
                // for this frame.
                ancestors.push(Ancestor::SvelteElement);
                walk_fragment_impl(&se.children, ctx, None, ancestors, false);
                ancestors.pop();
            }
            Node::IfBlock(b) => {
                crate::rules::block_rules::visit_if(b, ctx);
                walk_fragment_impl(&b.consequent, ctx, parent_tag, ancestors, true);
                for arm in &b.elseif_arms {
                    walk_fragment_impl(&arm.body, ctx, parent_tag, ancestors, true);
                }
                if let Some(else_body) = &b.alternate {
                    walk_fragment_impl(else_body, ctx, parent_tag, ancestors, true);
                }
            }
            Node::EachBlock(b) => {
                crate::rules::block_rules::visit_each(b, ctx);
                walk_fragment_impl(&b.body, ctx, parent_tag, ancestors, true);
                if let Some(empty) = &b.alternate {
                    walk_fragment_impl(empty, ctx, parent_tag, ancestors, true);
                }
            }
            Node::AwaitBlock(b) => {
                crate::rules::block_rules::visit_await(b, ctx);
                if let Some(pending) = &b.pending {
                    walk_fragment_impl(pending, ctx, parent_tag, ancestors, true);
                }
                if let Some(then) = &b.then_branch {
                    walk_fragment_impl(&then.body, ctx, parent_tag, ancestors, true);
                }
                if let Some(catch) = &b.catch_branch {
                    walk_fragment_impl(&catch.body, ctx, parent_tag, ancestors, true);
                }
            }
            Node::KeyBlock(b) => {
                crate::rules::block_rules::visit_key(b, ctx);
                walk_fragment_impl(&b.body, ctx, parent_tag, ancestors, true);
            }
            Node::SnippetBlock(b) => {
                // Snippet frames stop the placement checks (upstream
                // breaks at SnippetBlock) but not the a11y is_parent
                // walk.
                ancestors.push(Ancestor::Boundary);
                walk_fragment_impl(&b.body, ctx, parent_tag, ancestors, false);
                ancestors.pop();
            }
            Node::Text(t) => {
                crate::rules::text_rules::visit_text(t, ctx);
            }
            Node::Interpolation(_) | Node::Comment(_) => {}
        }

        if pushed {
            ctx.pop_ignore();
        }
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
