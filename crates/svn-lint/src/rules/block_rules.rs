//! Rules that fire on control-flow blocks.

use svn_parser::ast::{
    AwaitBlock, EachBlock, ElseIfArm, Fragment, IfBlock, Interpolation, KeyBlock, Node,
    SnippetBlock,
};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;

pub fn visit_if(b: &IfBlock, ctx: &mut LintContext<'_>) {
    visit_block_fragment_for_empty(&b.consequent, ctx);
    for arm in &b.elseif_arms {
        visit_block_fragment_for_empty(&arm.body, ctx);
    }
    if let Some(alt) = &b.alternate {
        visit_block_fragment_for_empty(alt, ctx);
    }
    if ctx.runes {
        validate_opening_tag(b.range.start, b'#', ctx);
    }
}

/// The opening-tag check of an `{:else if}` branch, which the compiler
/// visits as a nested `{#if}` once the branch before it is done.
pub fn visit_elseif(arm: &ElseIfArm, ctx: &mut LintContext<'_>) {
    if ctx.runes
        && let Some(start) = ctx.source[..arm.condition_range.start as usize].rfind('{')
    {
        validate_opening_tag(start as u32, b':', ctx);
    }
}

pub fn visit_each(b: &EachBlock, ctx: &mut LintContext<'_>) {
    validate_opening_tag(b.range.start, b'#', ctx);
    visit_block_fragment_for_empty(&b.body, ctx);
    if let Some(alt) = &b.alternate {
        visit_block_fragment_for_empty(alt, ctx);
    }
    // `EachBlock.js`: an item named `$state` or `$derived` is rejected
    // with the rune placement error, on the whole block.
    let context = b
        .as_clause
        .as_ref()
        .and_then(|c| c.context_range)
        .map(|r| r.slice(ctx.source))
        .map(|text| text.split(':').next().unwrap_or_default().trim());
    if let Some(name @ ("$state" | "$derived")) = context {
        ctx.emit_error(
            Code::state_invalid_placement,
            messages::state_invalid_placement(name),
            b.range,
        );
    }
    // A key makes the block keyed unless it is just the index; a keyed
    // block needs an `as` clause (`EachBlock.js`).
    if let Some(clause) = &b.as_clause
        && clause.context_range.is_none()
        && let Some(key) = clause.key_range
    {
        let key = trimmed(key, ctx.source);
        let index = clause.index_range.map(|r| trimmed(r, ctx.source));
        let key_text = key.slice(ctx.source);
        let keyed =
            index.is_none_or(|i| i.slice(ctx.source) != key_text) || !is_identifier(key_text);
        if keyed {
            ctx.emit_error(
                Code::each_key_without_as,
                messages::each_key_without_as(),
                key,
            );
        }
    }
}

pub fn visit_key(b: &KeyBlock, ctx: &mut LintContext<'_>) {
    visit_block_fragment_for_empty(&b.body, ctx);
    if ctx.runes {
        validate_opening_tag(b.range.start, b'#', ctx);
    }
}

pub fn visit_await(b: &AwaitBlock, ctx: &mut LintContext<'_>) {
    if let Some(pending) = &b.pending {
        visit_block_fragment_for_empty(pending, ctx);
    }
    if let Some(then) = &b.then_branch {
        visit_block_fragment_for_empty(&then.body, ctx);
    }
    if let Some(catch) = &b.catch_branch {
        visit_block_fragment_for_empty(&catch.body, ctx);
    }
    if ctx.runes {
        validate_opening_tag(b.range.start, b'#', ctx);
        // `{ :then v}` / `{ :catch e}`: whitespace between the brace and
        // the colon, found by looking just before the bound pattern.
        let values = [
            (
                b.then_branch.as_ref().and_then(|t| t.context_range),
                ":then",
            ),
            (
                b.catch_branch.as_ref().and_then(|c| c.context_range),
                ":catch",
            ),
        ];
        for (value, keyword) in values {
            if let Some(value) = value
                && let Some(start) = spaced_branch_brace(ctx.source, value.start, keyword)
            {
                ctx.emit_error(
                    Code::block_unexpected_character,
                    messages::block_unexpected_character(":"),
                    svn_core::Range::new(start, value.start),
                );
            }
        }
    }
}

/// `{#snippet}`'s checks before its body is visited: the opening tag
/// in runes mode, then no rest parameter (`SnippetBlock.js`).
pub fn visit_snippet(b: &SnippetBlock, ctx: &mut LintContext<'_>) {
    visit_block_fragment_for_empty(&b.body, ctx);
    if ctx.runes {
        validate_opening_tag(b.range.start, b'#', ctx);
    }
    if let Some(rest) = snippet_rest_parameter(b, ctx.source) {
        ctx.emit_error(
            Code::snippet_invalid_rest_parameter,
            messages::snippet_invalid_rest_parameter(),
            rest,
        );
    }
}

/// `{@const}` (`ConstTag.js`): the opening tag in runes mode, then its
/// placement — directly inside a block, a snippet, a component, a
/// `<svelte:fragment>` / `<svelte:boundary>`, or an element filling a
/// named slot.
/// A `{const …}` / `{let …}` declaration tag (the compiler's
/// `DeclarationTag` visitor): not in a legacy component, and at the top
/// level of the template not a name the instance script declares.
pub fn visit_declaration_tag(tag: &Interpolation, ctx: &mut LintContext<'_>) {
    if !ctx.runes && !ctx.maybe_runes {
        ctx.emit_error(
            Code::declaration_tag_no_legacy_mode,
            messages::declaration_tag_no_legacy_mode(),
            tag.range,
        );
    }
    if !ctx.template_path.is_empty() {
        return;
    }
    let Some(text) = ctx
        .source
        .get(tag.expression_range.start as usize..tag.expression_range.end as usize)
    else {
        return;
    };
    let wrapped = format!("let {text};");
    let offset = tag.expression_range.start as i64 - 4;
    let alloc = oxc_allocator::Allocator::default();
    let parsed = svn_parser::parse_script_body(&alloc, &wrapped, svn_parser::ScriptLang::Ts);
    let Some(oxc_ast::ast::Statement::VariableDeclaration(declaration)) =
        parsed.program.body.first()
    else {
        return;
    };
    let duplicate = declaration
        .declarations
        .iter()
        .flat_map(|d| {
            // The compiler's node for a declarator's plain name takes in
            // its type annotation.
            let typed_end = match &d.id {
                oxc_ast::ast::BindingPattern::BindingIdentifier(_) => {
                    d.type_annotation.as_ref().map(|a| a.span.end)
                }
                _ => None,
            };
            d.id.get_binding_identifiers()
                .into_iter()
                .map(move |id| (id, typed_end))
        })
        .find_map(|(id, typed_end)| {
            let at = |position: u32| (i64::from(position) + offset) as u32;
            let range = svn_core::Range::new(at(id.span.start), at(id.span.end));
            let tree = ctx.scope_tree.as_ref()?;
            let existing = tree.binding(
                *tree
                    .scope(tree.instance_root)
                    .declarations
                    .get(id.name.as_str())?,
            );
            // The tag's own name is not a clash with itself, and an
            // import belongs to the module's scope.
            (existing.range != range
                && existing.declaration_kind != crate::scope_types::DeclarationKind::Import)
                .then(|| {
                    let reported =
                        svn_core::Range::new(range.start, typed_end.map_or(range.end, at));
                    (id.name.to_string(), reported)
                })
        });
    if let Some((name, range)) = duplicate {
        ctx.emit_error(
            Code::declaration_duplicate,
            messages::declaration_duplicate(&name),
            range,
        );
    }
}

pub fn visit_const_tag(tag: &Interpolation, ctx: &mut LintContext<'_>) {
    use crate::walk::{ComponentKind, PathFrame};
    if ctx.runes {
        validate_opening_tag(tag.range.start, b'@', ctx);
    }
    let allowed = match ctx.template_path.last() {
        Some(
            PathFrame::IfBlock
            | PathFrame::EachBlock { .. }
            | PathFrame::AwaitBlock
            | PathFrame::KeyBlock
            | PathFrame::SnippetBlock
            | PathFrame::SvelteFragment
            | PathFrame::SvelteBoundary,
        ) => true,
        Some(PathFrame::Component { kind, .. }) => *kind != ComponentKind::SvelteSelf,
        Some(PathFrame::RegularElement { slotted, .. } | PathFrame::SvelteElement { slotted }) => {
            *slotted
        }
        Some(PathFrame::SvelteHead | PathFrame::Other) | None => false,
    };
    if !allowed {
        ctx.emit_error(
            Code::const_tag_invalid_placement,
            messages::const_tag_invalid_placement(),
            tag.range,
        );
    }
}

/// `{@html}` / `{@debug}` (`HtmlTag.js`, `DebugTag.js`): the opening
/// tag in runes mode.
pub fn visit_html_or_debug_tag(tag: &Interpolation, ctx: &mut LintContext<'_>) {
    if ctx.runes {
        validate_opening_tag(tag.range.start, b'@', ctx);
    }
}

/// `{@render}` (`RenderTag.js`): the opening tag, then no spread
/// argument and no `.bind` / `.apply` / `.call` callee.
pub fn visit_render_tag(tag: &Interpolation, ctx: &mut LintContext<'_>) {
    use oxc_ast::ast::{Argument, ChainElement, Expression};
    validate_opening_tag(tag.range.start, b'@', ctx);
    let range = tag.expression_range;
    let Some(text) = ctx.source.get(range.start as usize..range.end as usize) else {
        return;
    };
    let alloc = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let Ok(expr) = oxc_parser::Parser::new(&alloc, text, source_type).parse_expression() else {
        return;
    };
    let mut expr = &expr;
    while let Expression::ParenthesizedExpression(p) = expr {
        expr = &p.expression;
    }
    let call = match expr {
        Expression::CallExpression(call) => call,
        Expression::ChainExpression(chain) => match &chain.expression {
            ChainElement::CallExpression(call) => call,
            _ => return,
        },
        _ => return,
    };
    for arg in &call.arguments {
        if let Argument::SpreadElement(spread) = arg {
            ctx.emit_error(
                Code::render_tag_invalid_spread_argument,
                messages::render_tag_invalid_spread_argument(),
                svn_core::Range::new(
                    range.start + spread.span.start,
                    range.start + spread.span.end,
                ),
            );
        }
    }
    let mut callee = &call.callee;
    while let Expression::ParenthesizedExpression(p) = callee {
        callee = &p.expression;
    }
    if let Expression::StaticMemberExpression(member) = callee
        && matches!(member.property.name.as_str(), "bind" | "apply" | "call")
    {
        ctx.emit_error(
            Code::render_tag_invalid_call_expression,
            messages::render_tag_invalid_call_expression(),
            tag.range,
        );
    }
}

/// `validate_opening_tag`: the character right after a tag's `{` must
/// be the expected sigil (`{ #if}` is rejected), reported over the
/// first five characters.
fn validate_opening_tag(start: u32, expected: u8, ctx: &mut LintContext<'_>) {
    if ctx.source.as_bytes().get(start as usize + 1) != Some(&expected) {
        let end = (start as usize + 5).min(ctx.source.len());
        let end = (end..=ctx.source.len())
            .find(|&i| ctx.source.is_char_boundary(i))
            .unwrap_or(ctx.source.len());
        let expected = (expected as char).to_string();
        ctx.emit_error(
            Code::block_unexpected_character,
            messages::block_unexpected_character(&expected),
            svn_core::Range::new(start, end as u32),
        );
    }
}

/// For a `{:then}` / `{:catch}` pattern starting at `value`, the start
/// of the ten characters before it when they end in `{`, whitespace,
/// the keyword and whitespace (`/{(\s*):then\s+$/` with a non-empty
/// first group).
fn spaced_branch_brace(source: &str, value: u32, keyword: &str) -> Option<u32> {
    let mut from = (value as usize).saturating_sub(10);
    while !source.is_char_boundary(from) {
        from -= 1;
    }
    let window = &source[from..value as usize];
    let before_ws = window.trim_end_matches(svn_parser::is_svelte_whitespace);
    if before_ws.len() == window.len() {
        return None;
    }
    let before_keyword = before_ws.strip_suffix(keyword)?;
    let before_gap = before_keyword.trim_end_matches(svn_parser::is_svelte_whitespace);
    (before_gap.ends_with('{') && before_gap.len() != before_keyword.len()).then_some(from as u32)
}

fn trimmed(range: svn_core::Range, source: &str) -> svn_core::Range {
    let text = range.slice(source);
    let start = range.start + (text.len() - text.trim_start().len()) as u32;
    let end = range.end - (text.len() - text.trim_end().len()) as u32;
    svn_core::Range::new(start, end.max(start))
}

fn is_identifier(text: &str) -> bool {
    let mut chars = text.chars();
    chars
        .next()
        .is_some_and(|c| c.is_alphabetic() || c == '_' || c == '$')
        && chars.all(|c| c.is_alphanumeric() || c == '_' || c == '$')
}

/// The range of a rest element among a snippet's parameters, read as
/// the parameter list of an arrow function.
fn snippet_rest_parameter(b: &SnippetBlock, source: &str) -> Option<svn_core::Range> {
    let params = b.parameters_range;
    let text = source.get(params.start as usize..params.end as usize)?;
    if !text.contains("...") {
        return None;
    }
    let wrapped = format!("({text}) => {{}}");
    let alloc = oxc_allocator::Allocator::default();
    let source_type = oxc_span::SourceType::default()
        .with_module(true)
        .with_typescript(true);
    let expr = oxc_parser::Parser::new(&alloc, &wrapped, source_type)
        .parse_expression()
        .ok()?;
    let oxc_ast::ast::Expression::ArrowFunctionExpression(arrow) = &expr else {
        return None;
    };
    let rest = arrow.params.rest.as_ref()?;
    // `wrapped` has one byte (`(`) before the parameter text.
    Some(svn_core::Range::new(
        params.start + rest.span.start - 1,
        params.start + rest.span.end - 1,
    ))
}

/// The checks `SnippetBlock.js` makes once the snippet's body has been
/// visited: a snippet passed to a `<Component>` may not share its name
/// with a prop the component's attributes pass (`snippet_shadowing_prop`),
/// and an explicit `{#snippet children()}` conflicts with implicit
/// children content (`snippet_conflict`).
pub fn visit_snippet_after_body(b: &SnippetBlock, ctx: &mut LintContext<'_>) {
    // A snippet at the top level of the template may not take a name
    // the instance script declares. The compiler checks once it has
    // visited the snippet's body, so an error in there comes first.
    if ctx.template_path.is_empty()
        && let Some(tag) = ctx.source.get(b.range.start as usize..b.range.end as usize)
        && let Some(keyword) = tag.find("snippet")
        && let Some(name_at) = tag[keyword + 7..].find(b.name.as_str())
    {
        let start = b.range.start + (keyword + 7 + name_at) as u32;
        let range = svn_core::Range::new(start, start + b.name.len() as u32);
        let clashes = ctx.scope_tree.as_ref().is_some_and(|tree| {
            tree.scope(tree.instance_root)
                .declarations
                .get(b.name.as_str())
                .map(|existing| tree.binding(*existing))
                // An import belongs to the module's scope, not the
                // instance script's.
                .is_some_and(|existing| {
                    existing.range != range
                        && existing.declaration_kind != crate::scope_types::DeclarationKind::Import
                })
        });
        if clashes {
            ctx.emit_error(
                Code::declaration_duplicate,
                messages::declaration_duplicate(b.name.as_str()),
                range,
            );
        }
    }
    if let Some(crate::walk::PathFrame::Component {
        kind: crate::walk::ComponentKind::Component,
        props,
        ..
    }) = ctx.template_path.last()
        && props.contains(&b.name)
    {
        ctx.emit_error(
            Code::snippet_shadowing_prop,
            messages::snippet_shadowing_prop(&b.name),
            b.range,
        );
    }
    if b.name != "children" {
        return;
    }
    if let Some(crate::walk::PathFrame::Component {
        implicit_children: true,
        ..
    }) = ctx.template_path.last()
    {
        ctx.emit_error(
            Code::snippet_conflict,
            messages::snippet_conflict(),
            b.range,
        );
    }
}

/// `block_empty`: fires when a block body is exactly one
/// whitespace-only Text node (upstream heuristic — matches "user
/// hasn't started typing content yet").
fn visit_block_fragment_for_empty(frag: &Fragment, ctx: &mut LintContext<'_>) {
    let source = ctx.source;
    if frag.nodes.len() == 1
        && let Node::Text(t) = &frag.nodes[0]
        && t.range
            .slice(source)
            .chars()
            .all(svn_parser::is_svelte_whitespace)
    {
        let msg = messages::block_empty();
        ctx.emit(Code::block_empty, msg, t.range);
    }
}
