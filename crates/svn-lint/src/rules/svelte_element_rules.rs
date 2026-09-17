//! Rules that fire on `<svelte:*>` special elements.

use svn_parser::ast::{AttrValuePart, Attribute, SvelteElement, SvelteElementKind};

use crate::codes::Code;
use crate::context::LintContext;
use crate::messages;
use crate::rules::element_rules::{AttrParent, visit_attribute};

pub fn visit(se: &SvelteElement, ctx: &mut LintContext<'_>, ancestors: &[crate::walk::Ancestor]) {
    // svelte_self_invalid_placement (`SvelteSelf.js`): `<svelte:self>`
    // needs an `{#if}`, `{#each}`, `{#snippet}` or component ancestor.
    if se.kind == SvelteElementKind::SelfRef
        && !ctx.template_path.iter().any(|f| {
            matches!(
                f,
                crate::walk::PathFrame::IfBlock
                    | crate::walk::PathFrame::EachBlock
                    | crate::walk::PathFrame::SnippetBlock
                    | crate::walk::PathFrame::Component {
                        kind: crate::walk::ComponentKind::Component,
                        ..
                    }
            )
        })
    {
        ctx.emit_error(
            Code::svelte_self_invalid_placement,
            messages::svelte_self_invalid_placement(),
            se.range,
        );
    }

    // svelte_component_deprecated: `<svelte:component>` in runes mode.
    if ctx.runes && se.kind == SvelteElementKind::Component {
        let msg = messages::svelte_component_deprecated();
        ctx.emit(Code::svelte_component_deprecated, msg, se.range);
    }

    // svelte_self_deprecated: `<svelte:self>` in runes mode, naming the
    // component the way the compiler does from its file name.
    if ctx.runes && se.kind == SvelteElementKind::SelfRef {
        let (name, basename) = match &ctx.filename {
            Some(path) => (
                component_name(path, ctx.scope_tree.as_ref()),
                path.file_name()
                    .map(|b| b.to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
            None => ("Self".to_string(), "Self.svelte".to_string()),
        };
        let msg = messages::svelte_self_deprecated(&name, &basename);
        ctx.emit(Code::svelte_self_deprecated, msg, se.range);
    }

    // Attribute checks: which AttrParent shape to route through.
    let parent = match se.kind {
        SvelteElementKind::Component | SvelteElementKind::SelfRef => {
            AttrParent::SvelteComponentLike
        }
        SvelteElementKind::Element => AttrParent::SvelteElement,
        SvelteElementKind::Window => AttrParent::SvelteSpecial("svelte:window"),
        SvelteElementKind::Document => AttrParent::SvelteSpecial("svelte:document"),
        SvelteElementKind::Body => AttrParent::SvelteSpecial("svelte:body"),
        _ => AttrParent::OtherSvelte,
    };
    if matches!(parent, AttrParent::SvelteComponentLike) {
        crate::rules::component_rules::check_component_attributes(&se.attributes, ctx);
    }
    for attr in &se.attributes {
        visit_attribute(attr, ctx, parent);
    }

    // Only route `<svelte:element>` through the a11y check — the
    // other svelte:* kinds (component/self/window/document/body/
    // head/options/fragment/boundary) aren't rendered elements.
    if se.kind == SvelteElementKind::Element {
        // svelte_element_invalid_this: `<svelte:element this="div">` (or
        // `this="h{n}"`) — `this` should be a single `{expression}`, not a
        // string / text-with-interpolation. Mirrors upstream's
        // `!is_expression_attribute(this)` warning (1-parse/state/element.js).
        if let Some(this_attr) = se.attributes.iter().find(|a| match a {
            Attribute::Plain(p) => p.name == "this",
            Attribute::Expression(e) => e.name == "this",
            Attribute::Shorthand(s) => s.name == "this",
            _ => false,
        }) {
            let is_expression = match this_attr {
                Attribute::Expression(_) | Attribute::Shorthand(_) => true,
                Attribute::Plain(p) => matches!(
                    p.value.as_ref(),
                    Some(v) if v.parts.len() == 1
                        && matches!(v.parts[0], AttrValuePart::Expression { .. })
                ),
                _ => false,
            };
            if !is_expression {
                let r = match this_attr {
                    Attribute::Plain(p) => p.range,
                    Attribute::Expression(e) => e.range,
                    Attribute::Shorthand(s) => s.range,
                    _ => se.range,
                };
                ctx.emit(
                    Code::svelte_element_invalid_this,
                    messages::svelte_element_invalid_this(),
                    r,
                );
            }
        }
        crate::rules::a11y_rules::visit_dynamic(se, ctx, ancestors);
    }
}

/// The compiler's component name: `get_component_name` (the file's
/// base name without `.svelte`, or its directory for an `index`
/// outside `src`, capitalised), then `scope.generate` (characters an
/// identifier can't hold become `_`, and a name the component already
/// uses gets a `_N` suffix).
fn component_name(path: &std::path::Path, tree: Option<&crate::scope::ScopeTree>) -> String {
    let basename = path
        .file_name()
        .map(|b| b.to_string_lossy().into_owned())
        .unwrap_or_default();
    let mut name = basename.replacen(".svelte", "", 1);
    if name == "index"
        && let Some(dir) = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|d| d.to_string_lossy().into_owned())
        && !dir.is_empty()
        && dir != "src"
    {
        name = dir;
    }
    let mut chars = name.chars();
    let name: String = match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    };
    let mut preferred: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '$' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if preferred.starts_with(|c: char| c.is_ascii_digit()) {
        preferred.replace_range(0..1, "_");
    }
    let taken = |candidate: &str| {
        tree.is_some_and(|t| {
            t.all_bindings().any(|(_, b)| b.name == candidate)
                || t.unresolved_refs.iter().any(|r| r.name == candidate)
        })
    };
    if !taken(&preferred) {
        return preferred;
    }
    (1..)
        .map(|n| format!("{preferred}_{n}"))
        .find(|candidate| !taken(candidate))
        .unwrap_or(preferred)
}
