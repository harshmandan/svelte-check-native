//! Top-level walker stub.
//!
//! Connects the template AST from `svn-parser` to the lint rule
//! modules. Initial scaffold: just walks elements + components. Each
//! Phase expands this.

use std::path::Path;

use svn_parser::ast::{Attribute, Fragment, Node, SvelteElementKind};
use svn_parser::{parse_all_template_runs, parse_sections};

use crate::codes::Code;
use crate::context::{CustomElementInfo, LintContext};
use crate::messages;

/// Infer whether a file uses runes. Upstream heuristic:
/// - `.svelte.js` / `.svelte.ts` → runes mode
/// - `<svelte:options runes={…}>` → explicit override (resolved later
///   in the template walk)
/// - Any rune CALL (`$state(…)`, `$derived(…)`, …) anywhere in the
///   source → runes mode
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
pub fn infer_runes_mode(source: &str, path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.ends_with(".svelte.js") || name.ends_with(".svelte.ts") {
        return true;
    }
    // Only the script body content can contain real rune calls — a
    // commented-out `// $state(0)` example in the SAME script body
    // would still false-positive a raw byte scan, so we run the scan
    // inside a state machine that skips comments and string literals
    // (line, block, single, double, template). HTML content in the
    // template / comment block can also contain rune-shaped text but
    // the chance of `$state(` literally appearing there is low; we
    // still scope the scan to script bodies via parse_sections to
    // skip the template noise entirely.
    let (doc, _errors) = parse_sections(source);
    let mut scripts: Vec<&str> = Vec::new();
    if let Some(s) = &doc.module_script {
        scripts.push(s.content);
    }
    if let Some(s) = &doc.instance_script {
        scripts.push(s.content);
    }
    for content in scripts {
        if scan_script_for_rune_call(content) {
            return true;
        }
    }
    false
}

/// Find the byte offset of the `}` that closes a `${…}` interpolation
/// starting at `start` (the byte AFTER the opening `${`). Skips over
/// string literals (single, double, template), line/block comments,
/// and nested braces so the closing `}` is the structurally matching
/// one — not an unbalanced `}` that happens to appear inside a
/// string or comment.
///
/// Returns `None` when the interpolation is unterminated (truncated
/// source / parse-error region). Caller treats that as "scan to EOF
/// and stop." Regex literals are not currently distinguished — a
/// `/}/` inside an interpolation would slip past the `/` byte
/// without entering string mode, but the only fallout is a slightly
/// over-broad scan; rune detection is monotonic-OR so it never
/// produces a false negative.
fn find_interpolation_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut depth = 1usize;
    let mut j = start;
    while j < bytes.len() {
        let c = bytes[j];
        // Line comment.
        if c == b'/' && bytes.get(j + 1) == Some(&b'/') {
            while j < bytes.len() && bytes[j] != b'\n' {
                j += 1;
            }
            continue;
        }
        // Block comment.
        if c == b'/' && bytes.get(j + 1) == Some(&b'*') {
            j += 2;
            while j + 1 < bytes.len() && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                j += 1;
            }
            j = (j + 2).min(bytes.len());
            continue;
        }
        // Single/double-quoted string — escape-aware.
        if c == b'\'' || c == b'"' {
            j += 1;
            while j < bytes.len() && bytes[j] != c {
                if bytes[j] == b'\\' {
                    j = (j + 2).min(bytes.len());
                } else {
                    j += 1;
                }
            }
            j = (j + 1).min(bytes.len());
            continue;
        }
        // Nested template literal — recurse into its own interpolations
        // so a `}` inside a nested template's literal text can't
        // terminate the outer interpolation prematurely.
        if c == b'`' {
            j += 1;
            while j < bytes.len() && bytes[j] != b'`' {
                if bytes[j] == b'\\' {
                    j = (j + 2).min(bytes.len());
                    continue;
                }
                if bytes[j] == b'$' && bytes.get(j + 1) == Some(&b'{') {
                    let inner_start = j + 2;
                    j = match find_interpolation_end(bytes, inner_start) {
                        Some(pos) => (pos + 1).min(bytes.len()),
                        None => bytes.len(),
                    };
                    continue;
                }
                j += 1;
            }
            j = (j + 1).min(bytes.len());
            continue;
        }
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(j);
                }
            }
            _ => {}
        }
        j += 1;
    }
    None
}

/// Scan a JS/TS script body for any call-form rune occurrence
/// (`$state(`, `$derived(`, etc., or `$state.raw(` etc.). Skips line
/// comments, block comments, single/double-quoted strings, and
/// template-literal contents (re-entering when a `${…}` interpolation
/// opens). Returns true on the first match.
fn scan_script_for_rune_call(source: &str) -> bool {
    const MARKERS: &[&[u8]] = &[
        b"$state",
        b"$derived",
        b"$effect",
        b"$props",
        b"$bindable",
        b"$inspect",
        b"$host",
    ];
    let bytes = source.as_bytes();
    let mut i = 0;
    // Outer code state. Template-literal nesting depth tracked
    // separately so a `${…}` interpolation re-enters code-scan with
    // proper rune visibility.
    while i < bytes.len() {
        let b = bytes[i];
        // Line comment.
        if b == b'/' && bytes.get(i + 1) == Some(&b'/') {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        // Block comment.
        if b == b'/' && bytes.get(i + 1) == Some(&b'*') {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        // String literals — walk past, honouring `\\` escapes.
        if b == b'\'' || b == b'"' {
            let quote = b;
            i += 1;
            while i < bytes.len() && bytes[i] != quote {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                } else {
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            continue;
        }
        // Template literal — skip the literal text but recurse on
        // each `${…}` interpolation so a rune call inside one still
        // triggers detection.
        if b == b'`' {
            i += 1;
            while i < bytes.len() && bytes[i] != b'`' {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(bytes.len());
                    continue;
                }
                if bytes[i] == b'$' && bytes.get(i + 1) == Some(&b'{') {
                    // Recursively scan the interpolation's contents.
                    // Walk to the matching `}`, skipping strings,
                    // comments, regex literals, and nested template
                    // literals so an unbalanced `}` inside one of those
                    // can't terminate the interpolation early.
                    let interp_start = i + 2;
                    let j = find_interpolation_end(bytes, interp_start);
                    let end = j.unwrap_or(bytes.len());
                    let interp_text = &source[interp_start..end];
                    if scan_script_for_rune_call(interp_text) {
                        return true;
                    }
                    i = match j {
                        Some(pos) => (pos + 1).min(bytes.len()),
                        None => bytes.len(),
                    };
                    continue;
                }
                i += 1;
            }
            i = (i + 1).min(bytes.len());
            continue;
        }
        // Try matching a rune marker at this code position.
        for marker in MARKERS {
            if bytes[i..].starts_with(marker) {
                // Guard against `$$props` ambient: previous char must
                // not be `$`.
                let prev = i.checked_sub(1).and_then(|p| bytes.get(p)).copied();
                if prev != Some(b'$') {
                    let mut after = i + marker.len();
                    // Consume `.word` chains (`$state.raw`, etc.).
                    while bytes.get(after) == Some(&b'.') {
                        after += 1;
                        while after < bytes.len()
                            && (bytes[after].is_ascii_alphanumeric() || bytes[after] == b'_')
                        {
                            after += 1;
                        }
                    }
                    while after < bytes.len() && matches!(bytes[after], b' ' | b'\t') {
                        after += 1;
                    }
                    if bytes.get(after) == Some(&b'(') {
                        return true;
                    }
                }
            }
        }
        i += 1;
    }
    false
}

/// Walk a full `.svelte` source and run every phase-enabled rule.
///
/// Template parsing happens inline via `svn-parser`. Script parsing
/// happens later (Phase A's JS-side rules need the oxc AST).
pub fn walk(source: &str, ctx: &mut LintContext<'_>) {
    let (doc, _errors) = parse_sections(source);

    let (fragment, _parse_errors) = parse_all_template_runs(source, &doc.template.text_runs);

    // `<svelte:options runes>` / `<svelte:options runes={true}>` /
    // `<svelte:options runes={false}>` explicit override beats the
    // substring heuristic. Upstream: phase 2-analyze resolves this
    // from `root.options`.
    if let Some(explicit) = find_runes_option(&fragment, source) {
        ctx.runes = explicit;
    }

    // `<svelte:options customElement={…}>` wiring — drives
    // `options_missing_custom_element` (fires once for the attribute)
    // and `custom_element_props_identifier` (fires in
    // `binding_rules` per $props() identifier/rest candidate).
    // Upstream: `2-analyze/index.js:468-471, 688-690` + the
    // `VariableDeclarator.js:72-83` path. We don't receive compile
    // options, so `custom_element_from_option` is always false and
    // the attribute-presence alone triggers the missing-option
    // warning. `tag-custom-element-options-true` sets
    // `customElement: true` via `_config.js`; `upstream_validator`
    // already skips that fixture via the `_config.js` escape.
    if let Some((attr_range, has_props_option)) = find_custom_element_option(&fragment, source) {
        ctx.emit(
            Code::options_missing_custom_element,
            messages::options_missing_custom_element(),
            attr_range,
        );
        ctx.custom_element_info = Some(CustomElementInfo { has_props_option });
    }

    // Build the scope tree once; Phase-C rules query it by binding
    // name from both the script walker and the template walker. The
    // template walk here is what surfaces "identifier is referenced
    // in the template, not just in a script helper" to rules like
    // `non_reactive_update`.
    ctx.scope_tree = Some(crate::scope::build_with_template_and_runes(
        &doc,
        Some(&fragment),
        source,
        ctx.runes,
        ctx.compat,
    ));

    // <script>-attribute rules (script_unknown_attribute,
    // script_context_deprecated).
    crate::rules::script_rules::visit_document(&doc, ctx);

    // <script>-body (JS/TS AST) rules: perf_avoid_inline_class,
    // perf_avoid_nested_class, reactive_declaration_invalid_placement,
    // ...
    crate::rules::script_ast_rules::visit_document(&doc, ctx);

    // Phase-C binding-driven rules (non_reactive_update,
    // state_referenced_locally). Run AFTER script ast rules so
    // `ctx.scope_tree` is populated.
    crate::rules::binding_rules::visit(ctx);

    let mut ancestors: Vec<String> = Vec::new();
    walk_fragment_impl(&fragment, ctx, None, &mut ancestors, false);

    // element_implicitly_closed — source-level tag scanner. Runs
    // after the AST walk so it sits in a predictable output position.
    crate::rules::implicit_close::scan(source, ctx);
}

/// Scan the top-level fragment for `<svelte:options customElement={…}>`.
/// Returns the attribute's full range (matches upstream's warning
/// span: `customElement="..."` / `customElement={…}` including the
/// name) and whether the literal object has a `props` key, when the
/// value is an ObjectExpression. String / boolean values have no
/// props option.
fn find_custom_element_option(
    fragment: &Fragment,
    source: &str,
) -> Option<(svn_core::Range, bool)> {
    for node in &fragment.nodes {
        if let Node::SvelteElement(se) = node
            && se.kind == SvelteElementKind::Options
        {
            for attr in &se.attributes {
                match attr {
                    Attribute::Plain(p) if p.name.as_str() == "customElement" => {
                        // `customElement="name"` — string form. No props option.
                        return Some((p.range, false));
                    }
                    Attribute::Expression(e) if e.name.as_str() == "customElement" => {
                        // `customElement={expr}` — inspect the expression.
                        let expr_src = source.get(
                            e.expression_range.start as usize..e.expression_range.end as usize,
                        );
                        let has_props = expr_src
                            .map(object_expression_has_props_key)
                            .unwrap_or(false);
                        return Some((e.range, has_props));
                    }
                    _ => {}
                }
            }
        }
    }
    None
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

/// Scan the top-level fragment for `<svelte:options runes[={expr}]>`.
fn find_runes_option(fragment: &Fragment, source: &str) -> Option<bool> {
    for node in &fragment.nodes {
        if let Node::SvelteElement(se) = node
            && se.kind == SvelteElementKind::Options
        {
            for attr in &se.attributes {
                match attr {
                    Attribute::Plain(p) if p.name.as_str() == "runes" => {
                        // `runes` as a bare attribute or `runes="…"`
                        // evaluates truthily. Boolean literal text
                        // values `"false"` → false.
                        return Some(match &p.value {
                            None => true,
                            Some(v) => {
                                if v.parts.len() == 1 {
                                    if let svn_parser::ast::AttrValuePart::Text {
                                        content, ..
                                    } = &v.parts[0]
                                    {
                                        content.trim() != "false"
                                    } else {
                                        true
                                    }
                                } else {
                                    true
                                }
                            }
                        });
                    }
                    Attribute::Shorthand(s) if s.name.as_str() == "runes" => {
                        return Some(true);
                    }
                    Attribute::Expression(e) if e.name.as_str() == "runes" => {
                        // `runes={expr}` — read the expression's source
                        // bytes and recognise the literal `true` / `false`
                        // forms. Anything else (variable refs, function
                        // calls) → conservative truthy fallback. Without
                        // the literal-false carve-out, opting out via
                        // `runes={false}` was silently treated as opt-IN
                        // and the file got linted under runes-mode rules.
                        let start = e.expression_range.start as usize;
                        let end = e.expression_range.end as usize;
                        let trimmed = source.get(start..end).map(str::trim).unwrap_or("");
                        return Some(match trimmed {
                            "false" => false,
                            "true" => true,
                            _ => true,
                        });
                    }
                    _ => {}
                }
            }
        }
    }
    None
}

/// Recursively visit every template node, dispatching rules as we go.
///
/// `parent_tag`: closest enclosing regular-element tag, for
/// `is_tag_valid_with_parent` checks.
/// `ancestors`: stack of enclosing regular-element tags (outer → inner),
/// for `is_tag_valid_with_ancestor` checks.
/// `inside_control_block`: true if we're currently inside an
/// `{#if}`/`{#each}`/`{#await}`/`{#key}`. Only in that case does
/// the placement warning fire (otherwise upstream errors).
fn walk_fragment_impl(
    fragment: &Fragment,
    ctx: &mut LintContext<'_>,
    parent_tag: Option<&str>,
    ancestors: &mut Vec<String>,
    inside_control_block: bool,
) {
    for node in &fragment.nodes {
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
            Node::Text(t) => t.content.chars().any(|c| !c.is_whitespace()),
            _ => false,
        };
        let ignores = if is_target {
            crate::ignore::collect_preceding_comment_ignores(&fragment.nodes, node, ctx)
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
                ancestors.push(el.name.to_string());
                walk_fragment_impl(
                    &el.children,
                    ctx,
                    Some(el.name.as_str()),
                    ancestors,
                    inside_control_block,
                );
                ancestors.pop();
            }
            Node::Component(comp) => {
                crate::rules::component_rules::visit(comp, ctx);
                // Components reset the ancestor chain — upstream
                // breaks out when it sees a Component ancestor.
                let mut empty_ancestors: Vec<String> = Vec::new();
                walk_fragment_impl(&comp.children, ctx, None, &mut empty_ancestors, false);
            }
            Node::SvelteElement(se) => {
                crate::rules::svelte_element_rules::visit(se, ctx, ancestors);
                let mut empty_ancestors: Vec<String> = Vec::new();
                walk_fragment_impl(&se.children, ctx, None, &mut empty_ancestors, false);
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
                // Snippets reset the ancestor chain.
                let mut empty_ancestors: Vec<String> = Vec::new();
                walk_fragment_impl(&b.body, ctx, parent_tag, &mut empty_ancestors, false);
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

    use super::infer_runes_mode;
    use std::path::PathBuf;

    fn p(name: &str) -> PathBuf {
        PathBuf::from(name)
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
}

#[cfg(test)]
mod runes_options_tests {
    #![allow(clippy::expect_used, clippy::unwrap_used)]

    use super::find_runes_option;
    use svn_parser::{parse_all_template_runs, parse_sections};

    fn detect(src: &str) -> Option<bool> {
        let (doc, _) = parse_sections(src);
        let (fragment, _) = parse_all_template_runs(src, &doc.template.text_runs);
        find_runes_option(&fragment, src)
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
